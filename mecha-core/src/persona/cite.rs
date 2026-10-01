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
//! **What was received is read off a flat rendering.** The walk splits a
//! text at `document: <name> ·` headers and `=== page N of M ·` markers,
//! so a line inside a page that spells one of those starts a "file" or
//! "page" of its own. Only material the owner put in the folders can do
//! it, and the worst it buys is a badge or a page attributed to the wrong
//! name — never a quote admitted that the chat did not receive.
//!
//! **Words are split at spaces.** A script written without them (Chinese,
//! Japanese, Thai) is one word per paragraph here, so a quote from such a
//! file is "not in the file" whatever it says — the check cannot speak for
//! it, and the badge then overstates. And a quote is checked against a
//! page, or a page and the next: one that spans three is not found.
//!
//! A citation is found by its brackets, on one line: a quote holding a
//! bracket of its own (a reference mark, "as shown [4]") or wrapped across
//! a line is not parsed, and goes untagged — unchecked, which is what an
//! untagged citation says.
//!
//! **A citation is not entailment.** A quote that is there can still be
//! made to support the wrong claim; the page says "quoted, not checked for
//! support", which is all this establishes. And the check is the harness's,
//! never a tool: a check the model may decline to call is not a check.

use crate::grounding::{self, Claim};
use crate::message::{Block, Message, Role};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The tools whose results are a file's rendered text, as `files::read` and
/// `document_read` render it: a `document: <name> · …` header, then pages.
/// `file_search` renders its passages the same way (`persona::search`).
const READERS: [&str; 3] = ["file_read", "document_read", "file_search"];

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
    /// One of its files, listed to the chat, whose text the chat had not
    /// read when it quoted it — the quote may be invented, the file is not
    /// (review of #465, pass 5).
    NotRead,
    /// A file in a script written without spaces: its words cannot be told
    /// apart here, so the check cannot speak for the quote either way.
    CannotCheck,
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
    /// `text` through [`normal`], made once and dropped when `text` grows.
    norm: Option<String>,
}

/// What the chat had received of its files, built up message by message:
/// the files block in its first turn, and every reader result the model
/// read ([`grounding::calls`]: first seen wins, a stale or failed result is
/// never evidence).
struct Received<'m> {
    read: HashMap<&'m str, &'m str>,
    pages: Vec<Page>,
    /// Files the files block named without their text (too long to include,
    /// still being read, read on request).
    listed: Vec<String>,
}

impl<'m> Received<'m> {
    fn new(messages: &'m [Message]) -> Self {
        let read = grounding::calls(messages)
            .into_iter()
            .filter(|c| READERS.contains(&c.name))
            .filter_map(|c| Some((c.id, c.result?)))
            .collect();
        Received {
            read,
            pages: Vec::new(),
            listed: Vec::new(),
        }
    }

    /// Take in what one message delivered.
    fn absorb(&mut self, m: &Message) {
        if m.role != Role::User {
            return;
        }
        for block in &m.content {
            let text = match block {
                // The same predicate as `files::carries` and
                // `Taint::arm_for_content` (review of #465).
                Block::Text { text } if text.trim_start().starts_with(super::files::FILES_STEM) => {
                    self.listed.extend(text.lines().filter_map(|l| {
                        Some(l.strip_prefix("- `")?.strip_suffix('`')?.to_string())
                    }));
                    text.as_str()
                }
                Block::ToolResult { tool_use_id, .. } => {
                    match self.read.get(tool_use_id.as_str()) {
                        Some(text) => text,
                        None => continue,
                    }
                }
                _ => continue,
            };
            for page in documents(text) {
                self.add(page);
            }
        }
    }

    /// A page received twice — the files block and a later `file_read` of
    /// it, or text layer and OCR — is one page, whatever came between, so a
    /// file is named once and a citation without its `@group/` resolves
    /// (review of #465). The same text twice is kept once, so the page
    /// shows it once.
    fn add(&mut self, p: Page) {
        let Some(m) = self
            .pages
            .iter_mut()
            .find(|m| m.file == p.file && m.page == p.page)
        else {
            self.pages.push(p);
            return;
        };
        let (have, new) = (m.text.trim(), p.text.trim());
        if have.contains(new) {
            return;
        }
        if new.contains(have) {
            m.text = p.text;
        } else {
            m.text.push('\n');
            m.text.push_str(&p.text);
        }
        m.norm = None;
    }

    /// Every file received, each once.
    fn files(&self) -> Vec<&str> {
        let mut files: Vec<&str> = self.pages.iter().map(|p| p.file.as_str()).collect();
        files.sort_unstable();
        files.dedup();
        files
    }
}

/// Everything the chat received of its files, by the end of `messages`.
fn received(messages: &[Message]) -> Vec<Page> {
    let mut r = Received::new(messages);
    for m in messages {
        r.absorb(m);
    }
    r.pages
}

/// The documents a rendered text holds, split at each `document: <name> ·`
/// header and each document at its `=== page N of M ·` markers.
fn documents(text: &str) -> Vec<Page> {
    let mut out = Vec::new();
    let mut current: Option<(String, Option<u32>, String)> = None;
    let flush = |cur: Option<(String, Option<u32>, String)>, out: &mut Vec<Page>| {
        if let Some((file, page, text)) = cur {
            out.push(Page {
                file,
                page,
                text,
                norm: None,
            });
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
    // Before a paged document's first marker is its header matter (the
    // layout stage's notice, say), not a page: kept, it would answer for
    // every cited page and mark nothing (review of #465, pass 4). A text
    // file has no markers, and its body is its one page.
    let paged: Vec<String> = out
        .iter()
        .filter(|p| p.page.is_some())
        .map(|p| p.file.clone())
        .collect();
    out.retain(|p| p.page.is_some() || !paged.contains(&p.file));
    out
}

/// The pages of one rendered document — `files::read`'s or a reader's
/// output — as (page, text), the page `None` for a text file: the same
/// split the check reads with, so a search passage carries the page a
/// citation of it will be checked against (`persona::search`).
pub(crate) fn pages_of(text: &str) -> Vec<(Option<u32>, String)> {
    documents(text)
        .into_iter()
        .map(|p| (p.page, p.text))
        .collect()
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

/// Lowercase, with typographic quotes made plain, a hyphen, soft hyphen,
/// en dash or minus a `-`, an em dash a space (it separates words), and
/// ligatures spelled out.
fn plain(s: &str) -> String {
    let mut plain = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\u{2018}' | '\u{2019}' | '\u{201b}' | '\u{2032}' => plain.push('\''),
            '\u{201c}' | '\u{201d}' | '\u{201f}' | '\u{2033}' => plain.push('"'),
            // A figure or en dash joins what it ranges over ("2019–2025"),
            // as a hyphen does, so a model's "2019-2025" is the same word
            // (review of #465, pass 4); an em dash breaks words.
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2212}' => plain.push('-'),
            '\u{2014}' | '\u{2015}' => plain.push(' '),
            '\u{fb00}' => plain.push_str("ff"),
            '\u{fb01}' => plain.push_str("fi"),
            '\u{fb02}' => plain.push_str("fl"),
            '\u{fb03}' => plain.push_str("ffi"),
            '\u{fb04}' => plain.push_str("ffl"),
            // A soft hyphen is a hyphen where it shows — at a line break —
            // so `tokens` can join across it; mid-word it is dropped as any
            // hyphen is (review of #465, pass 6).
            '\u{ad}' => plain.push('-'),
            c => plain.extend(c.to_lowercase()),
        }
    }
    plain
}

/// The words of `text` as the check compares them, each with the byte span
/// of the text it came from — one tokeniser for both [`normal`] and
/// [`mark`], so what the check admits is what the page marks.
///
/// A word is [`plain`], with the punctuation around it dropped and **every
/// hyphen inside it dropped**: "Anglo-Saxon", "Anglo-\nSaxon" broken at a
/// line and "anglosaxon" are one word, and so are "hold-\nfasts" and
/// "holdfasts". A PDF breaks lines wherever it likes, and no rule about the
/// letter after the break can tell a compound's hyphen from the line's
/// (review of #465: one that tried was dead code, `plain` having already
/// lowercased the letter it looked at).
fn tokens(text: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut spans = text
        .split_whitespace()
        .map(|w| {
            let start = w.as_ptr() as usize - text.as_ptr() as usize;
            (start, start + w.len())
        })
        .peekable();
    while let Some((start, mut end)) = spans.next() {
        let mut joined = plain(&text[start..end]);
        // A word broken at a line by a hyphen is one word.
        while joined.ends_with('-') {
            let Some(&(next, next_end)) = spans.peek() else {
                break;
            };
            let gap = &text[end..next];
            if !(gap.starts_with('\n') && gap[1..].chars().all(|c| c == ' ' || c == '\t')) {
                break;
            }
            spans.next();
            joined.push_str(&plain(&text[next..next_end]));
            end = next_end;
        }
        // An em dash became a space: it may part one span in two. A hyphen
        // between digits parts them too — "2019-2025", "2019–2025" and
        // "2019 – 2025" are one run of words (review of #465, pass 5) —
        // and anywhere else it is dropped, joining the word.
        let chars: Vec<char> = joined.chars().collect();
        let mut parted = String::with_capacity(joined.len());
        for (i, &ch) in chars.iter().enumerate() {
            if ch != '-' {
                parted.push(ch);
            } else if i > 0
                && chars[i - 1].is_ascii_digit()
                && chars.get(i + 1).is_some_and(char::is_ascii_digit)
            {
                parted.push(' ');
            }
        }
        for part in parted.split_whitespace() {
            let w = part.trim_matches(|c: char| !c.is_alphanumeric());
            if !w.is_empty() {
                out.push((start, end, w.to_string()));
            }
        }
    }
    out
}

/// What both sides go through before containment: [`tokens`]' words,
/// joined by single spaces.
fn normal(s: &str) -> String {
    tokens(s)
        .into_iter()
        .map(|t| t.2)
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
///
/// In conversation order, one per citation as written: the page pairs them
/// with the citations on screen counting from the end (`citeEntries` in
/// `persona.js`), so reordering these misattributes every badge.
pub fn check_conversation(messages: &[Message]) -> Vec<Checked> {
    let mut received = Received::new(messages);
    let mut out = Vec::new();
    for m in messages {
        if m.role != Role::Assistant {
            received.absorb(m);
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
        for c in parse(&text) {
            out.push(check(&c, &mut received));
        }
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

/// One received page, normalised, as `admit` reads a referent.
struct View<'a> {
    id: String,
    text: &'a str,
}

impl grounding::Referent for View<'_> {
    fn id(&self) -> &str {
        &self.id
    }
    fn text(&self) -> &str {
        self.text
    }
}

fn check(c: &Citation, received: &mut Received<'_>) -> Checked {
    let cited = c.pages.map(|p| p.0);
    let checked = |file: &str, verdict| Checked {
        raw: c.raw.clone(),
        file: file.to_string(),
        cited,
        quote: c.quote.clone(),
        verdict,
    };
    let Some(file) = resolve(&c.file, &received.files()).map(str::to_string) else {
        let listed: Vec<&str> = received.listed.iter().map(String::as_str).collect();
        return match resolve(&c.file, &listed) {
            Some(file) => checked(file, Verdict::NotRead),
            None => checked(&c.file, Verdict::NoSuchFile),
        };
    };
    let quote = normal(&c.quote);
    for p in received.pages.iter_mut().filter(|p| p.file == file) {
        if p.norm.is_none() {
            p.norm = Some(normal(&p.text));
        }
    }
    let views: Vec<View> = received
        .pages
        .iter()
        .filter(|p| p.file == file)
        .map(|p| View {
            id: p.page.map(|n| n.to_string()).unwrap_or_default(),
            text: p.norm.as_deref().unwrap_or_default(),
        })
        .collect();
    // A script without spaces is a handful of "words" a paragraph: nothing
    // here can say whether a quote is in it (review of #465, pass 5).
    let (chars, words) = views.iter().fold((0usize, 0usize), |(c, w), v| {
        (c + v.text.chars().count(), w + v.text.split(' ').count())
    });
    if chars > 200 && chars > words * 30 {
        return checked(&file, Verdict::CannotCheck);
    }
    // Decided before any page is searched, so it is the verdict wherever the
    // cited page is (review of #465: a page never received skipped it) —
    // and after the script check, a short quote in a script without spaces
    // being a sentence, not a coincidence.
    if quote.chars().count() < MIN_QUOTE_CHARS {
        return checked(&file, Verdict::TooShort);
    }
    let in_range = |v: &View| match (c.pages, v.id.parse::<u32>().ok()) {
        (Some((a, b)), Some(n)) => (a..=b).contains(&n),
        // No page cited, or a text file with none: the file is the page.
        _ => true,
    };
    // `admit` is the lookup; whole words on top of it, as `mark` compares,
    // so "holdfast" is not quoted out of "holdfasts" and every quote the
    // check admits is one the page can mark (review of #465).
    let padded = format!(" {quote} ");
    let quoted = |v: &View| {
        grounding::admit(
            &claim(&c.raw, &v.id, &quote),
            std::slice::from_ref(v),
            MIN_QUOTE_CHARS,
        )
        .is_ok()
            // Both sides are `tokens` joined by single spaces, so a padded
            // match is a match of whole words.
            && format!(" {} ", v.text).contains(&padded)
    };
    // The page cited first, then the rest of the file.
    if let Some(v) = views.iter().filter(|v| in_range(v)).find(|v| quoted(v)) {
        return checked(
            &file,
            Verdict::Quoted {
                found: v.id.parse().ok(),
            },
        );
    }
    if let Some(found) = views
        .iter()
        .filter(|v| !in_range(v))
        .find(|v| quoted(v))
        .and_then(|v| v.id.parse().ok())
    {
        return checked(&file, Verdict::OtherPage { found });
    }
    // Last, a quote that runs over a page break: each page with the next,
    // found on the first of the two (review of #465, pass 4: it was "not
    // in the file", the harshest verdict, for a sentence that is).
    let mut paged: Vec<(u32, &str)> = views
        .iter()
        .filter_map(|v| Some((v.id.parse().ok()?, v.text)))
        .collect();
    paged.sort_by_key(|p| p.0);
    for pair in paged.windows(2) {
        let ((a, first), (b, second)) = (pair[0], pair[1]);
        if b != a + 1 || !format!(" {first} {second} ").contains(&padded) {
            continue;
        }
        let cited_here = c
            .pages
            .is_none_or(|(x, y)| (x..=y).contains(&a) || (x..=y).contains(&b));
        return checked(
            &file,
            if cited_here {
                Verdict::Quoted { found: Some(a) }
            } else {
                Verdict::OtherPage { found: a }
            },
        );
    }
    checked(&file, Verdict::NotFound)
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

/// What a citation opens: `page` of `file` as the chat received it, with
/// where `quote` sits in it — or, for a quote the check found running over
/// the page break (`Verdict::Quoted` from the pair), that page and the next
/// joined, so the passage it admitted is the passage marked (review of
/// #465, pass 5: the page alone marked nothing).
pub fn passage(
    messages: &[Message],
    file: &str,
    page: Option<u32>,
    quote: &str,
) -> Option<Passage> {
    let text = page_text(messages, file, page)?;
    if let Some(span) = mark(&text, quote) {
        return Some(Passage {
            text,
            span: Some(span),
            through: page,
        });
    }
    if let Some(next) = page.and_then(|p| {
        let n = p + 1;
        Some((n, page_text(messages, file, Some(n))?))
    }) {
        let joined = format!("{text}\n\n{}", next.1);
        if let Some(span) = mark(&joined, quote) {
            return Some(Passage {
                text: joined,
                span: Some(span),
                through: Some(next.0),
            });
        }
    }
    Some(Passage {
        text,
        span: None,
        through: page,
    })
}

/// A cited page as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passage {
    pub text: String,
    /// Where the quote sits in `text`, as byte offsets; `None` when it
    /// could not be placed.
    pub span: Option<(usize, usize)>,
    /// The last page `text` holds: the cited one, or the next where the
    /// quote ran over the break.
    pub through: Option<u32>,
}

/// Where `quote` sits in `text`, as byte offsets of its first and last
/// word — the words compared as the check compares them, so the passage it
/// admitted is the passage marked. `None` when it is not there.
pub fn mark(text: &str, quote: &str) -> Option<(usize, usize)> {
    let words = tokens(text);
    let wanted = tokens(quote);
    if wanted.is_empty() {
        return None;
    }
    words
        .windows(wanted.len())
        .find(|run| run.iter().map(|w| &w.2).eq(wanted.iter().map(|w| &w.2)))
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
                // Listed to the chat, not yet read (review of #465, pass 5).
                ("@kelp/survey.pdf", &Verdict::NotRead),
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
        // Received twice, shown once (review of #465, pass 2).
        let page = page_text(&msgs, "survey.pdf", Some(4)).unwrap();
        assert_eq!(page.matches("run monthly").count(), 1, "{page}");
    }

    /// Review of #465, pass 2: whole words, as `mark` compares. "holdfast"
    /// is not quoted out of "holdfasts" — which would be badged quoted with
    /// nothing on the page to mark — and a short quote is too short to
    /// check whichever page it cites, received or not.
    #[test]
    fn a_partial_word_is_not_quoted_and_short_is_short_on_any_page() {
        let msgs = chat(
            PAPER,
            "[kelp.pdf, p. 1: \"urchins graze kelp holdfast\"] \
             [kelp.pdf, p. 1: \"rchins graze kelp holdfasts\"] \
             [kelp.pdf, p. 7: \"at night\"]",
        );
        let got: Vec<Verdict> = check_conversation(&msgs)
            .into_iter()
            .map(|c| c.verdict)
            .collect();
        assert_eq!(
            got,
            [Verdict::NotFound, Verdict::NotFound, Verdict::TooShort]
        );
        // And what the check admits, the page can mark.
        let page = page_text(&msgs, "kelp.pdf", Some(1)).unwrap();
        assert!(mark(&page, "urchins graze kelp holdfasts").is_some());
        assert!(mark(&page, "urchins graze kelp holdfast").is_none());
    }

    /// Review of #465, pass 3: a compound broken at a line is the same
    /// word as the model's copy of it, whatever case follows the break —
    /// and an em dash parts words rather than gluing them.
    #[test]
    fn a_compound_broken_at_a_line_is_still_the_same_words() {
        let page = "document: hist.pdf \u{b7} pdf \u{b7} 1 page(s) \u{b7} sha256 x\n\
            \n=== page 1 of 1 \u{b7} text layer (the file's own words) ===\n\
            The Anglo-\nSaxon chronicle is a well-\nknown source\u{2014}mostly.\n";
        let msgs = chat(
            page,
            "[hist.pdf, p. 1: \"The Anglo-Saxon chronicle is a well-known source\"] \
             [hist.pdf, p. 1: \"a well-known source - mostly\"]",
        );
        let got: Vec<Verdict> = check_conversation(&msgs)
            .into_iter()
            .map(|c| c.verdict)
            .collect();
        assert_eq!(
            got,
            [
                Verdict::Quoted { found: Some(1) },
                Verdict::Quoted { found: Some(1) },
            ]
        );
        let text = page_text(&msgs, "hist.pdf", Some(1)).unwrap();
        let (a, b) = mark(&text, "Anglo-Saxon chronicle").unwrap();
        assert_eq!(&text[a..b], "Anglo-\nSaxon chronicle");
    }

    /// Review of #465, pass 4: the page's en dash is the model's hyphen; a
    /// sentence over a page break is quoted, not "not in the file"; and a
    /// paged document's preamble is no page of its own.
    #[test]
    fn a_range_dash_a_page_break_and_a_preamble_are_read_right() {
        let doc = "document: kelp.pdf \u{b7} pdf \u{b7} 2 page(s) \u{b7} sha256 x\n\
            (the layout stage is unavailable, so OCR pages below were read whole)\n\
            \n=== page 1 of 2 \u{b7} text layer (the file's own words) ===\n\
            Transects ran 2019\u{2013}2025 at six sites, and the\n\
            \n=== page 2 of 2 \u{b7} text layer (the file's own words) ===\n\
            counts fell every winter.\n";
        let msgs = chat(
            doc,
            "[kelp.pdf, p. 1: \"Transects ran 2019-2025 at six sites\"] \
             [kelp.pdf, p. 1: \"six sites, and the counts fell every winter\"] \
             [kelp.pdf, p. 2: \"layout stage is unavailable\"]",
        );
        let got: Vec<Verdict> = check_conversation(&msgs)
            .into_iter()
            .map(|c| c.verdict)
            .collect();
        assert_eq!(
            got,
            [
                Verdict::Quoted { found: Some(1) },
                Verdict::Quoted { found: Some(1) },
                Verdict::NotFound,
            ]
        );
    }

    /// Review of #465, pass 5: what the check admits across a page break
    /// is what the page marks; a spaced range dash is the model's hyphen;
    /// a listed file not yet read is "not read", not "no such file"; and a
    /// script without spaces is "can't check", not "not in the file".
    #[test]
    fn what_the_check_cannot_speak_for_it_does_not_accuse() {
        let doc = "document: kelp.pdf \u{b7} pdf \u{b7} 2 page(s) \u{b7} sha256 x\n\
            \n=== page 1 of 2 \u{b7} text layer (the file's own words) ===\n\
            Transects ran 2019 \u{2013} 2025 at six sites, and the\n\
            \n=== page 2 of 2 \u{b7} text layer (the file's own words) ===\n\
            counts fell every winter.\n";
        let block =
            format!("{doc}\nNot included \u{2014} read them with `file_read`:\n- `survey.pdf`");
        let msgs = chat(
            &block,
            "[kelp.pdf, p. 1: \"Transects ran 2019-2025 at six sites\"] \
             [kelp.pdf, p. 1: \"six sites, and the counts fell every winter\"] \
             [survey.pdf, p. 3: \"an invented sentence about otters\"]",
        );
        let got: Vec<Verdict> = check_conversation(&msgs)
            .into_iter()
            .map(|c| c.verdict)
            .collect();
        assert_eq!(
            got,
            [
                Verdict::Quoted { found: Some(1) },
                Verdict::Quoted { found: Some(1) },
                Verdict::NotRead,
            ]
        );
        let Passage {
            text,
            span,
            through: last,
        } = passage(
            &msgs,
            "kelp.pdf",
            Some(1),
            "six sites, and the counts fell every winter",
        )
        .unwrap();
        let (a, b) = span.expect("marked across the break");
        assert!(
            text[a..b].starts_with("six sites") && text[a..b].ends_with("winter."),
            "{:?}",
            &text[a..b]
        );
        assert_eq!(last, Some(2));

        let cjk = format!(
            "document: \u{6587}.pdf \u{b7} pdf \u{b7} 1 page(s) \u{b7} sha256 x\n\
             \n=== page 1 of 1 \u{b7} text layer (the file's own words) ===\n{}\n",
            "\u{6d77}\u{80c6}\u{98df}\u{6d77}\u{85fb}".repeat(60)
        );
        let msgs = chat(
            &cjk,
            "[\u{6587}.pdf, p. 1: \"\u{6d77}\u{80c6}\u{98df}\u{6d77}\u{85fb}\u{306f}\"]",
        );
        assert_eq!(check_conversation(&msgs)[0].verdict, Verdict::CannotCheck);
    }

    /// Review of #465, pass 6: a page that hyphenates with a soft hyphen at
    /// a line break is the same word as the model's copy of it.
    #[test]
    fn a_soft_hyphen_at_a_line_break_joins_the_word() {
        let page = "document: kelp.pdf \u{b7} pdf \u{b7} 1 page(s) \u{b7} sha256 x\n\
            \n=== page 1 of 1 \u{b7} text layer (the file's own words) ===\n\
            Sea urchins graze kelp hold\u{ad}\nfasts at night.\n";
        let msgs = chat(
            page,
            "[kelp.pdf, p. 1: \"urchins graze kelp holdfasts at night\"]",
        );
        assert_eq!(
            check_conversation(&msgs)[0].verdict,
            Verdict::Quoted { found: Some(1) }
        );
    }
}
