//! Semantic views of untrusted envelope fields. Raw strings remain in the
//! message and ledger; a failed decode is missing evidence, never literal text.
use base64::{engine::general_purpose::STANDARD, Engine as _};

/// Unfold continuation lines and canonicalize horizontal whitespace. A newline
/// without continuation whitespace is not a header value we can interpret.
pub(super) fn unfold(raw: &str) -> Option<String> {
    let mut chars = raw.chars().peekable();
    let mut text = String::new();
    while let Some(c) = chars.next() {
        match c {
            '\r' | '\n' => {
                if c == '\r' && chars.next()? != '\n' {
                    return None;
                }
                if !chars.peek().is_some_and(|c| matches!(c, ' ' | '\t')) {
                    return None;
                }
                if !text.ends_with(' ') {
                    text.push(' ');
                }
            }
            ' ' | '\t' => {
                if !text.ends_with(' ') {
                    text.push(' ');
                }
            }
            c if c.is_control() => return None,
            c => text.push(c),
        }
    }
    Some(text)
}

/// RFC 2047 B/Q words, with strict byte/charset validation. This runs only on
/// raw-header values, never on Graph's already decoded subject. Adjacent encoded
/// words ignore intervening folding whitespace, as required by RFC 2047 §6.2.
pub(super) fn decode(raw: &str) -> Option<String> {
    let unfolded = unfold(raw)?;
    let mut rest = unfolded.as_str();
    let mut text = String::new();
    let mut previous_encoded = false;
    while let Some(start) = rest.find("=?") {
        let prefix = &rest[..start];
        if !(previous_encoded && prefix.trim().is_empty()) {
            text.push_str(prefix);
        }
        rest = &rest[start + 2..];
        let (charset, tail) = rest.split_once('?')?;
        let (encoding, tail) = tail.split_once('?')?;
        let (encoded, tail) = tail.split_once("?=")?;
        if charset.len() + encoding.len() + encoded.len() + 6 > 75
            || encoded.is_empty()
            || !encoded.bytes().all(|b| b.is_ascii_graphic() && b != b'?')
        {
            return None;
        }
        let bytes = match encoding.to_ascii_lowercase().as_str() {
            "b" => STANDARD.decode(encoded).ok()?,
            "q" => {
                let mut bytes = Vec::new();
                let mut input = encoded.bytes();
                while let Some(b) = input.next() {
                    bytes.push(match b {
                        b'_' => b' ',
                        b'=' => {
                            let high = char::from(input.next()?).to_digit(16)?;
                            let low = char::from(input.next()?).to_digit(16)?;
                            (high * 16 + low) as u8
                        }
                        b => b,
                    });
                }
                bytes
            }
            _ => return None,
        };
        let word = match charset.to_ascii_lowercase().as_str() {
            "utf-8" | "utf8" => String::from_utf8(bytes).ok()?,
            "us-ascii" | "ascii" if bytes.is_ascii() => String::from_utf8(bytes).ok()?,
            "iso-8859-1" | "iso8859-1" | "latin-1" | "latin1" => {
                bytes.into_iter().map(char::from).collect()
            }
            _ => return None,
        };
        if word.chars().any(char::is_control) {
            return None;
        }
        text.push_str(&word);
        previous_encoded = true;
        rest = tail;
    }
    text.push_str(rest);
    unfold(&text)
}

fn atom(c: char) -> bool {
    c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~".contains(c)
}

fn quoted_end(input: &str, comment: bool) -> Option<usize> {
    let mut depth = 1usize;
    let mut escaped = false;
    for (i, c) in input.char_indices().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '(' if comment => depth += 1,
            ')' if comment => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            '"' if !comment => return Some(i + 1),
            _ => {}
        }
    }
    None
}

/// RFC 2919 identifier shape, accepting trailing comments as an interoperability
/// extension. Only the bracketed dot-atom is the identity. Whitespace inside
/// brackets is ignored (§3); malformed syntax cannot establish a negative.
pub(super) fn list_id(raw: &str) -> Option<String> {
    let unfolded = unfold(raw.trim())?;
    let mut rest = unfolded.trim();
    loop {
        rest = rest.trim_start();
        match rest.chars().next()? {
            '<' => break,
            '(' => rest = &rest[quoted_end(rest, true)?..],
            '"' => rest = &rest[quoted_end(rest, false)?..],
            c if atom(c) || c == '.' => {
                let end = rest
                    .find(|c: char| !atom(c) && c != '.')
                    .unwrap_or(rest.len());
                rest = &rest[end..];
            }
            _ => return None,
        }
    }
    let (id, mut tail) = rest.strip_prefix('<')?.split_once('>')?;
    let id: String = id.chars().filter(|c| !matches!(c, ' ' | '\t')).collect();
    if id.len() > 255
        || !id.contains('.')
        || !id
            .split('.')
            .all(|part| !part.is_empty() && part.chars().all(atom))
    {
        return None;
    }
    loop {
        tail = tail.trim_start();
        if tail.is_empty() {
            return Some(id);
        }
        if !tail.starts_with('(') {
            return None;
        }
        tail = &tail[quoted_end(tail, true)?..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_words_decode_charsets_and_adjacent_words_strictly() {
        assert_eq!(decode("=?UTF-8?B?aW52b2ljZQ==?="), Some("invoice".into()));
        assert_eq!(decode("=?utf-8?q?caf=C3=A9?="), Some("café".into()));
        assert_eq!(decode("=?latin-1?Q?caf=E9?="), Some("café".into()));
        assert_eq!(
            decode("=?us-ascii?Q?in?=\r\n\t=?us-ascii?B?dm9pY2U=?="),
            Some("invoice".into())
        );
        assert_eq!(
            decode("=?us-ascii?Q?invoice_ready?="),
            Some("invoice ready".into())
        );
        for raw in [
            "=?us-ascii?Q?=FF?=",
            "=?utf-8?Q?=C3?=",
            "=?latin-1?Q?=0A?=",
            "=?utf-8?B?!!?=",
            "=?utf-8?Q?in voice?=",
            "=?utf-8?X?invoice?=",
            "=?utf-8?Q?unfinished",
            "invoice\r\nnot-a-continuation",
        ] {
            assert_eq!(decode(raw), None, "{raw}");
        }
    }
}
