//! What a call says aloud: a reply's text tidied for speech as it streams.
//!
//! The rule is Listen's (`web/src/lib/speech.js` `speakable`): Markdown's
//! marks go and their words stay, a link is its label, a bare URL is "a
//! link", a code block is announced and never read, a persona's citation is
//! where it came from. Applied here, where every call's words leave mecha
//! (`pump`, both chats), so a reply is written one way whether the owner
//! reads it or hears it (the owner's ask, 2026-10-06), and the voice never
//! says the formatting the chat draws. `web/test/speakable-cases.json` holds
//! the two to one rule; both suites read it.
//!
//! Streaming is the point. A call speaks a sentence as soon as it is written,
//! so the tidier releases each line as it ends, and within a line each
//! sentence end with no mark left open (a link, a citation, bold, code) —
//! the speech engine waits for a sentence end anyway, so nothing is heard
//! later than before. A mark left open holds the line, never past
//! [`MAX_HELD`]. A code fence is announced the moment it opens and skipped
//! until it closes.
//!
//! Hand-rolled rather than regex: the crate has no regex dependency, and
//! each rule is one scan.

/// What a code block is spoken as.
const CODE_BLOCK: &str = "There is a code block here.";

/// The most of a line held waiting for a sentence end with nothing open: a
/// mark the model never closes (a lone `*`) must not silence a paragraph.
/// Past it, the line goes out at its last space.
pub(crate) const MAX_HELD: usize = 600;

/// A reply's text in, as deltas; speakable text out, as soon as it is safe.
#[derive(Debug)]
pub(crate) struct Tidier {
    held: String,
    /// `held` begins a line, so a heading, quote or list marker is a mark.
    line_start: bool,
    /// Inside a code fence: everything is dropped until it closes.
    in_code: bool,
}

impl Default for Tidier {
    fn default() -> Self {
        Self {
            held: String::new(),
            line_start: true,
            in_code: false,
        }
    }
}

impl Tidier {
    /// Take a delta; return what can be spoken now (possibly nothing). Each
    /// piece ends in a space, so pieces join as the reply did.
    pub(crate) fn push(&mut self, delta: &str) -> String {
        self.held.push_str(delta);
        let mut out = String::new();
        loop {
            if self.in_code {
                match self.held.find("```") {
                    Some(i) => {
                        self.held.drain(..i + 3);
                        self.in_code = false;
                        // What follows a fence is a line of its own, as the
                        // page's rule has it.
                        self.line_start = true;
                        continue;
                    }
                    None => {
                        // Keep only what could begin the closing fence.
                        let ticks = self.held.len() - self.held.trim_end_matches('`').len();
                        let keep = ticks.min(2);
                        self.held.drain(..self.held.len() - keep);
                        break;
                    }
                }
            }
            let fence = self.held.find("```");
            let newline = self.held.find('\n');
            match (fence, newline) {
                (Some(f), n) if n.is_none_or(|n| f < n) => {
                    let line = self.held[..f].to_string();
                    speak(&mut out, &line, self.line_start, true);
                    out.push_str(CODE_BLOCK);
                    out.push(' ');
                    self.held.drain(..f + 3);
                    self.in_code = true;
                }
                (_, Some(n)) => {
                    let line = self.held[..n].to_string();
                    speak(&mut out, &line, self.line_start, true);
                    self.held.drain(..=n);
                    self.line_start = true;
                }
                (_, None) => {
                    if let Some((cut, spaced)) = safe_cut(&self.held, self.line_start) {
                        let part = self.held[..cut].to_string();
                        // Cut where the text had no space: join as written.
                        if speak(&mut out, &part, self.line_start, false) && !spaced {
                            out.pop();
                        }
                        self.held.drain(..cut);
                        self.line_start = false;
                    }
                    break;
                }
            }
        }
        out
    }

    /// The end of a turn's text: whatever is held, spoken as a line's end.
    /// The tidier is ready for the next turn after it.
    pub(crate) fn finish(&mut self) -> String {
        let mut out = String::new();
        if !self.in_code {
            let rest = std::mem::take(&mut self.held);
            speak(&mut out, &rest, self.line_start, true);
        }
        *self = Self::default();
        out
    }
}

/// `text` whole, as a call would say it: the rule the fixture states, and
/// what a blocking (non-streaming) completion answers with.
pub(crate) fn speakable(text: &str) -> String {
    let mut t = Tidier::default();
    let mut s = t.push(text);
    s.push_str(&t.finish());
    collapse(&s)
}

/// Sentence ends, in scripts with spaces and the full-width marks of those
/// without (Chinese, Japanese), which end a sentence with no space after.
const ENDS: [char; 6] = ['.', '!', '?', '。', '！', '？'];
const WIDE_ENDS: [char; 3] = ['。', '！', '？'];

/// Where `held` (one line, no newline yet) may be cut, and whether the cut
/// falls at a space (so the pieces join with one): just past the last
/// sentence end with nothing open before it, beyond any line marker. Past
/// [`MAX_HELD`], the last space, or with none — a script written without
/// spaces — all of it (review of #574: the valve needed a space to open, so
/// such a reply was silent to its end).
fn safe_cut(held: &str, line_start: bool) -> Option<(usize, bool)> {
    let min = if line_start { marker_len(held) } else { 0 };
    for (i, c) in held.char_indices().rev().filter(|&(i, _)| i > min) {
        if c.is_whitespace() {
            let before = held[..i].trim_end_matches(['"', '\'', '”', '’', ')', ']']);
            if before.ends_with(ENDS) && closed(&held[..i]) {
                return Some((i + c.len_utf8(), true));
            }
        } else if WIDE_ENDS.contains(&c) {
            // Only with words after it: a mark that ends what is held may
            // still have a closing quote or a space to come.
            let end = i + c.len_utf8();
            if end < held.len()
                && !held[end..].starts_with(char::is_whitespace)
                && closed(&held[..end])
            {
                return Some((end, false));
            }
        }
    }
    if held.len() > MAX_HELD {
        let space = held
            .char_indices()
            .rev()
            .find(|&(i, c)| c.is_whitespace() && i > min)
            .map(|(i, c)| (i + c.len_utf8(), true));
        return space.or(Some((held.len(), false)));
    }
    None
}

/// No mark is left open in `s`: cutting after it splits nothing.
fn closed(s: &str) -> bool {
    let even = |n: usize| n.is_multiple_of(2);
    let doubles = s.matches("**").count();
    let singles = s.matches('*').count() - 2 * doubles;
    let open_bracket = match (s.rfind('['), s.rfind(']')) {
        (Some(o), Some(c)) => o > c,
        (Some(_), None) => true,
        _ => false,
    };
    let open_target = s.rfind("](").is_some_and(|k| !s[k..].contains(')'));
    even(s.matches('`').count())
        && even(doubles)
        && even(singles)
        && even(s.matches("~~").count())
        && {
            let (pairs, lone) = underscore_marks(s);
            even(pairs) && even(lone)
        }
        && !open_bracket
        && !open_target
}

/// Underscore marks that can be emphasis, as `__` pairs and lone `_`, the
/// way `*` is counted: an open `__` is one pair, never two lone marks that
/// look closed (review of #574). A run inside a word (`snake_case`) is the
/// word's and counts as neither.
fn underscore_marks(s: &str) -> (usize, usize) {
    let chars: Vec<char> = s.chars().collect();
    let (mut pairs, mut lone) = (0, 0);
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '_' {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i] == '_' {
            i += 1;
        }
        let before = start > 0 && chars[start - 1].is_alphanumeric();
        let after = chars.get(i).is_some_and(|c| c.is_alphanumeric());
        if !(before && after) {
            pairs += (i - start) / 2;
            lone += (i - start) % 2;
        }
    }
    (pairs, lone)
}

/// Tidy one piece of a line and append it to `out`, saying whether it
/// wrote anything (a piece of marks alone writes nothing). `line_start`: it begins
/// the line, so a marker is a mark. `eol`: it ends the line, so it ends a
/// sentence when heard.
fn speak(out: &mut String, piece: &str, line_start: bool, eol: bool) -> bool {
    let mut s = citations(piece);
    s = images_and_links(&s);
    s = bare_urls(&s);
    s = unwrap(&s, "`", |inner, _, _| !inner.contains('`'));
    if line_start {
        s = s[marker_len(&s)..].to_string();
        if is_rule(&s) {
            s.clear();
        }
    }
    s = unwrap(&s, "**", |_, _, _| true);
    s = unwrap(&s, "__", |_, _, _| true);
    s = unwrap(&s, "*", |inner, _, _| edges_not_space(inner));
    s = unwrap(&s, "_", |inner, before, after| {
        edges_not_space(inner)
            && !before.is_some_and(char::is_alphanumeric)
            && !after.is_some_and(char::is_alphanumeric)
    });
    s = unwrap(&s, "~~", |_, _, _| true);
    s = s.replace('|', " ");
    let s = collapse(&s);
    if s.is_empty() || s.chars().all(|c| c == '-' || c == ':' || c == ' ') {
        return false;
    }
    out.push_str(&s);
    if eol && !s.ends_with(['.', '!', '?', ':', ';', ')', '。', '！', '？']) {
        out.push('.');
    }
    out.push(' ');
    true
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn edges_not_space(inner: &str) -> bool {
    inner.chars().next().is_some_and(|c| !c.is_whitespace())
        && inner.chars().last().is_some_and(|c| !c.is_whitespace())
}

/// The length of a line's leading heading, quote and list markers, taken in
/// that order (`# `, `> `, `- ` or `1. `).
fn marker_len(line: &str) -> usize {
    let mut at = 0;
    // A heading: up to three spaces, one to six `#`, then space.
    let indent = |s: &str, most: usize| s.len() - s.trim_start_matches(' ').len() <= most;
    let rest = &line[at..];
    if indent(rest, 3) {
        let body = rest.trim_start_matches(' ');
        let hashes = body.len() - body.trim_start_matches('#').len();
        let after = &body[hashes..];
        if (1..=6).contains(&hashes) && after.starts_with(char::is_whitespace) {
            at += rest.len() - after.trim_start().len();
        }
    }
    // A quote: up to three spaces, `>`, one optional space.
    let rest = &line[at..];
    if indent(rest, 3) {
        let body = rest.trim_start_matches(' ');
        if let Some(after) = body.strip_prefix('>') {
            let after = after.strip_prefix(' ').unwrap_or(after);
            at += rest.len() - after.len();
        }
    }
    // A list marker: any indent, `-`, `*`, `+`, or digits then `.` or `)`,
    // then space.
    let rest = &line[at..];
    let body = rest.trim_start();
    let marker = if body.starts_with(['-', '*', '+']) {
        1
    } else {
        let digits = body.len() - body.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits > 0 && body[digits..].starts_with(['.', ')']) {
            digits + 1
        } else {
            0
        }
    };
    if marker > 0 && body[marker..].starts_with(char::is_whitespace) {
        let after = body[marker..].trim_start();
        at += rest.len() - after.len();
    }
    at
}

/// A thematic break: three or more `-`, `*` or `_` alone on the line.
fn is_rule(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3 && ['-', '*', '_'].iter().any(|&m| t.chars().all(|c| c == m))
}

/// `[file, p. N: "quote"]` and `[file: "quote"]` as "(from file words)".
fn citations(s: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    while let Some(rel) = s[i..].find('[') {
        let open = i + rel;
        out.push_str(&s[i..open]);
        let inner_end = s[open + 1..].find([']', '\n']).map(|k| open + 1 + k);
        match inner_end {
            Some(close) if s[close..].starts_with(']') => {
                if let Some(file) = cite_file(&s[open + 1..close]) {
                    out.push_str(&format!("(from {})", file_words(file)));
                    i = close + 1;
                    continue;
                }
            }
            _ => {}
        }
        out.push('[');
        i = open + 1;
    }
    out.push_str(&s[i..]);
    out
}

/// The file a citation's inside names, if it is one.
fn cite_file(inner: &str) -> Option<&str> {
    let body = inner.strip_suffix('"')?;
    let quote_at = body.rfind('"')?;
    let head = body[..quote_at].trim_end().strip_suffix(':')?;
    // An optional `, p. N`.
    let mut file = head;
    let digits = head.trim_end_matches(|c: char| c.is_ascii_digit());
    if digits.len() < head.len() {
        if let Some(p) = digits.trim_end().strip_suffix("p.") {
            if let Some(f) = p.trim_end().strip_suffix(',') {
                file = f;
            }
        }
    }
    (!file.is_empty()).then_some(file)
}

/// A cited file as words: no `@scope/`, no extension, `-` and `_` as spaces.
fn file_words(file: &str) -> String {
    let mut f = file;
    if f.starts_with('@') {
        if let Some(k) = f.find('/') {
            f = &f[k + 1..];
        }
    }
    if let Some(dot) = f.rfind('.') {
        let ext = &f[dot + 1..];
        if (1..=5).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
            f = &f[..dot];
        }
    }
    let spaced: String = f
        .chars()
        .map(|c| if c == '-' || c == '_' { ' ' } else { c })
        .collect();
    collapse(&spaced)
}

/// `![alt](src)` as "a picture: alt", `[label](href)` as its label.
fn images_and_links(s: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    while let Some(rel) = s[i..].find('[') {
        let open = i + rel;
        let image = open > i && s[..open].ends_with('!');
        let label_end = s[open + 1..].find(']').map(|k| open + 1 + k);
        let target = label_end.and_then(|c| {
            let after = &s[c + 1..];
            after
                .starts_with('(')
                .then(|| after.find(')').map(|k| c + 1 + k))
                .flatten()
        });
        if let (Some(close), Some(end)) = (label_end, target) {
            let label = &s[open + 1..close];
            if image {
                out.push_str(&s[i..open - 1]);
                out.push_str("a picture");
                if !label.is_empty() {
                    out.push_str(": ");
                    out.push_str(label);
                }
                i = end + 1;
                continue;
            }
            if !label.is_empty() {
                out.push_str(&s[i..open]);
                out.push_str(label);
                i = end + 1;
                continue;
            }
        }
        out.push_str(&s[i..=open]);
        i = open + 1;
    }
    out.push_str(&s[i..]);
    out
}

/// What may close a sentence or a bracket right after an address.
const URL_TAIL: [char; 12] = ['.', ',', '!', '?', ';', ':', '\'', '"', ')', ']', '”', '’'];

/// A bare `http(s)://` address, up to the next space less the punctuation
/// that ends it, as "a link".
fn bare_urls(s: &str) -> String {
    let mut out = String::new();
    let mut i = 0;
    loop {
        let next = ["https://", "http://"]
            .iter()
            .filter_map(|p| s[i..].find(p).map(|k| i + k))
            .min();
        let Some(at) = next else { break };
        let word_before = s[..at]
            .chars()
            .last()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if word_before {
            out.push_str(&s[i..at + 1]);
            i = at + 1;
            continue;
        }
        out.push_str(&s[i..at]);
        let end = s[at..]
            .find(char::is_whitespace)
            .map_or(s.len(), |k| at + k);
        // Up to the next space, less the punctuation that ends it: the
        // sentence's own stop is not the address's (review of #574 — "Read
        // https://a.io/x. It's good." was one sentence with "a link" in it).
        let scheme = if s[at..].starts_with("https://") {
            8
        } else {
            7
        };
        let url = s[at..end].trim_end_matches(URL_TAIL);
        if url.len() > scheme {
            out.push_str("a link");
            i = at + url.len();
        } else {
            // A scheme with nothing after it is no address.
            out.push_str(&s[at..end]);
            i = end;
        }
    }
    out.push_str(&s[i..]);
    out
}

/// Every `mark…mark` pair whose inside `ok` accepts, as its inside alone.
/// The nearest acceptable closing mark wins, as a lazy pattern's would. `ok`
/// sees the inside and the characters just outside the pair.
fn unwrap(s: &str, mark: &str, ok: impl Fn(&str, Option<char>, Option<char>) -> bool) -> String {
    let mut out = String::new();
    let mut i = 0;
    'scan: while let Some(rel) = s[i..].find(mark) {
        let open = i + rel;
        let start = open + mark.len();
        let before = s[..open].chars().last();
        let mut from = start;
        while let Some(rel) = s.get(from..).and_then(|r| r.find(mark)) {
            let close = from + rel;
            let inner = &s[start..close];
            let after = s[close + mark.len()..].chars().next();
            if !inner.is_empty() && ok(inner, before, after) {
                out.push_str(&s[i..open]);
                out.push_str(inner);
                i = close + mark.len();
                continue 'scan;
            }
            from = close + s[close..].chars().next().map_or(1, char::len_utf8);
        }
        let step = open + s[open..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&s[i..step]);
        i = step;
    }
    out.push_str(&s[i..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct Case {
        why: String,
        text: String,
        spoken: String,
    }

    fn cases() -> Vec<Case> {
        let raw = include_str!("../../../web/test/speakable-cases.json");
        let cases: Vec<Case> = serde_json::from_str(raw).expect("the shared fixture parses");
        assert!(!cases.is_empty());
        cases
    }

    /// The call says what Listen says: one rule, one fixture, two suites.
    #[test]
    fn a_call_speaks_by_listens_rule() {
        for c in cases() {
            assert_eq!(speakable(&c.text), c.spoken, "{}", c.why);
        }
    }

    /// However the reply arrives — a character at a time, or in any chunk
    /// size — what is spoken is the same as the whole.
    #[test]
    fn streaming_never_changes_what_is_said() {
        for c in cases() {
            let chars: Vec<char> = c.text.chars().collect();
            for size in 1..=7 {
                let mut t = Tidier::default();
                let mut said = String::new();
                for chunk in chars.chunks(size) {
                    said.push_str(&t.push(&chunk.iter().collect::<String>()));
                }
                said.push_str(&t.finish());
                assert_eq!(collapse(&said), c.spoken, "{} (chunks of {size})", c.why);
            }
        }
    }

    /// A sentence is released when it ends, not held for the line: the
    /// call's first words must not wait for the paragraph.
    #[test]
    fn a_sentence_goes_as_soon_as_it_ends() {
        let mut t = Tidier::default();
        assert_eq!(t.push("Hello **there**."), "");
        assert_eq!(t.push(" How was"), "Hello there. ");
        assert_eq!(t.push(" the ferry? It"), "How was the ferry? ");
        assert_eq!(t.finish(), "It. ");
        // A smile is no open mark.
        let mut t = Tidier::default();
        assert_eq!(
            t.push("Hi :) Good to see you. And"),
            "Hi :) Good to see you. "
        );
    }

    /// An open mark holds the line — a link's label is never spoken as
    /// "[the" — and a mark never closed holds it only so long.
    #[test]
    fn an_open_mark_holds_and_only_so_long() {
        let mut t = Tidier::default();
        assert_eq!(t.push("See [the guide. It"), "");
        assert_eq!(
            t.push(" helps](https://example.com/g). Then"),
            "See the guide. It helps. "
        );
        let mut t = Tidier::default();
        let long = format!("A lone * opens nothing. {}", "word ".repeat(MAX_HELD / 5));
        let out = t.push(&long);
        assert!(!out.is_empty(), "a never-closed mark silenced the line");
    }

    /// A script written without spaces ends a sentence at its own mark and
    /// is released there, joined as written; and a long run of it with no
    /// mark at all still goes once it passes [`MAX_HELD`] (review of #574).
    #[test]
    fn a_reply_without_spaces_is_not_held_to_its_end() {
        let mut t = Tidier::default();
        assert_eq!(t.push("你好。再"), "你好。");
        assert_eq!(t.push("见"), "");
        assert_eq!(t.finish(), "再见. ");
        let mut t = Tidier::default();
        assert!(
            !t.push(&"字".repeat(MAX_HELD)).is_empty(),
            "the valve never opened"
        );
    }

    /// A list's number is a marker, never a sentence end of its own.
    #[test]
    fn a_list_number_is_not_a_sentence() {
        let mut t = Tidier::default();
        assert_eq!(t.push("1. "), "");
        assert_eq!(t.push("Pack the bag. Then"), "Pack the bag. ");
    }

    /// A code block is announced when it opens, and its lines are never
    /// spoken, however long it runs.
    #[test]
    fn a_code_block_is_announced_and_skipped() {
        let mut t = Tidier::default();
        assert_eq!(t.push("Try:\n```sh\n"), "Try: There is a code block here. ");
        assert_eq!(t.push("echo one\necho two\n``"), "");
        assert_eq!(t.push("`\nDone"), "");
        assert_eq!(t.finish(), "Done. ");
    }

    /// `finish` leaves the tidier as new: a turn's open fence or held line
    /// never reaches the next turn.
    #[test]
    fn finish_resets() {
        let mut t = Tidier::default();
        t.push("```\nunclosed");
        assert_eq!(t.finish(), "");
        assert_eq!(t.push("New turn.\n"), "New turn. ");
    }
}
