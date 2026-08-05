//! Byte-safe splitting of an SSE stream into complete text segments.
//!
//! Network chunk boundaries are arbitrary: a multi-byte UTF-8 character can
//! arrive half in one chunk and half in the next. Decoding each chunk on its
//! own (`String::from_utf8_lossy` per chunk) turns that split character into
//! replacement characters — on the success path, corrupting both the deltas
//! shown to the user and the text accumulated into the transcript. SSE
//! delimiters are ASCII, so the fix is to split on *bytes* first and decode
//! only complete segments: a multi-byte sequence never contains an ASCII
//! newline, so a complete segment is complete UTF-8.

#[derive(Default)]
pub(crate) struct SseBuffer {
    buf: Vec<u8>,
}

impl SseBuffer {
    pub fn push(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
    }

    /// The next segment ending in `delim` (delimiter included), or `None`
    /// until one is complete. Decoding is lossy on purpose: a server that
    /// truly emits invalid UTF-8 should garble one character, not kill a run
    /// mid-answer.
    pub fn next_segment(&mut self, delim: &[u8]) -> Option<String> {
        let idx = self.buf.windows(delim.len()).position(|w| w == delim)?;
        let segment: Vec<u8> = self.buf.drain(..idx + delim.len()).collect();
        Some(String::from_utf8_lossy(&segment).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_character_split_across_chunks_survives_intact() {
        // "é" is 0xC3 0xA9. The old per-chunk decode turned this split into
        // two replacement characters — in the delta and in the transcript.
        let mut buf = SseBuffer::default();
        buf.push(b"data: {\"t\":\"\xc3");
        assert_eq!(buf.next_segment(b"\n"), None, "no complete line yet");
        buf.push(b"\xa9\"}\n");
        assert_eq!(buf.next_segment(b"\n").unwrap(), "data: {\"t\":\"é\"}\n");
        assert_eq!(buf.next_segment(b"\n"), None);
    }

    #[test]
    fn frames_split_on_the_two_byte_delimiter() {
        let mut buf = SseBuffer::default();
        buf.push(b"event: x\ndata: 1\n\ndata: 2\n\ntail");
        assert_eq!(buf.next_segment(b"\n\n").unwrap(), "event: x\ndata: 1\n\n");
        assert_eq!(buf.next_segment(b"\n\n").unwrap(), "data: 2\n\n");
        assert_eq!(
            buf.next_segment(b"\n\n"),
            None,
            "the tail waits for its delimiter"
        );
    }

    #[test]
    fn genuinely_invalid_utf8_degrades_to_replacement_rather_than_failing() {
        let mut buf = SseBuffer::default();
        buf.push(b"bad \xff byte\n");
        let line = buf.next_segment(b"\n").unwrap();
        assert!(line.contains('\u{FFFD}'));
        assert!(line.starts_with("bad "));
    }
}
