//! Where a warning goes while a full-screen front-end owns the terminal.
//!
//! `tracing` is configured in `main` to write to stderr, and under `mecha tui`
//! stderr **is the alternate screen ratatui is painting**. A single
//! `tracing::warn!` from anywhere in core — there are seventy such sites, and
//! the default filter is `warn`, so they are on for everyone — writes its
//! bytes at whatever cursor position crossterm last left, straight through the
//! frame. It then stays there forever: ratatui repaints by diffing against its
//! own buffer, so it believes those cells already hold what it wants and never
//! touches them again. That is the whole of "things are not clearing and we
//! get collisions" — the scribble is not a layout bug at all, and no amount of
//! redrawing can remove it.
//!
//! Three rules, each of which is a bug if undone:
//!
//! - **Nothing is dropped silently.** A captured line becomes a transcript
//!   entry, and anything still held when the screen is handed back is printed
//!   to the real stderr on the way out. A warning routed to `/dev/null`
//!   because it was inconvenient to display is the silently-degrading shape
//!   this project keeps finding: the run that finished on a failed tool call
//!   would look exactly like the run that did not.
//! - **The buffer is bounded, and says when it truncated.** `MECHA_LOG=debug`
//!   is thousands of lines a minute, and a queue nobody drains fast enough is
//!   a memory leak with a UI. Oldest go first and the count of what went is
//!   reported, because a list that silently looks shorter is the thing the
//!   outbox modal's hidden-item counter exists to avoid.
//! - **Capture is off by default.** Every other front-end — `run`, `chat`,
//!   the triggers daemon, the MCP binaries — wants stderr, and a writer that
//!   swallowed by default would take their diagnostics with it. Only a caller
//!   that has actually taken the screen turns it on.
//!
//! Known latency, stated rather than hidden: the loop drains at the top of
//! each iteration, so a line lands immediately during a run (the tick is
//! 200ms) and within one idle tick — up to a minute — when nothing else is
//! happening. Waking the loop would mean a channel the writer sends on, and
//! at idle there is nothing running to warn about, so the complexity buys a
//! delay nobody is waiting through. Any keypress drains it at once.

use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};

/// The most lines held before the oldest start falling off.
const CAP: usize = 2_000;

#[derive(Default)]
struct Buffer {
    /// `None` means nobody has taken the screen: writes go to stderr, which
    /// is what every non-TUI front-end wants.
    lines: Option<Vec<String>>,
    /// Bytes written since the last newline. `tracing`'s formatter does not
    /// promise one `write` per event, so a line has to be reassembled rather
    /// than assumed.
    partial: String,
    /// How many lines fell off the front to stay under `CAP`, since the last
    /// drain. Reported, never swallowed.
    dropped: usize,
}

fn buffer() -> &'static Mutex<Buffer> {
    static B: OnceLock<Mutex<Buffer>> = OnceLock::new();
    B.get_or_init(Mutex::default)
}

/// Start holding log lines instead of writing them to the terminal. Call it
/// once the alternate screen is up; `release` gives the terminal back.
pub fn capture() {
    if let Ok(mut b) = buffer().lock() {
        b.lines.get_or_insert_with(Vec::new);
    }
}

/// Take everything held so far, leaving capture on.
pub fn drain() -> Vec<String> {
    let Ok(mut b) = buffer().lock() else {
        return Vec::new();
    };
    let dropped = std::mem::take(&mut b.dropped);
    let mut out = b.lines.as_mut().map(std::mem::take).unwrap_or_default();
    if dropped > 0 {
        out.insert(
            0,
            format!("{dropped} earlier log line(s) dropped — the buffer is {CAP} lines"),
        );
    }
    out
}

/// Stop capturing and hand back whatever was still held, including a line that
/// never got its newline. The caller prints these *after* leaving the alternate
/// screen — a warning that arrived in the last frame is still a warning.
pub fn release() -> Vec<String> {
    let mut out = drain();
    if let Ok(mut b) = buffer().lock() {
        let tail = std::mem::take(&mut b.partial);
        if !tail.trim().is_empty() {
            out.push(tail.trim_end().to_string());
        }
        b.lines = None;
    }
    out
}

/// The `tracing` writer. Public only so `main` can install it.
pub struct Writer;

impl Write for Writer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // A poisoned lock must not lose the message, and stderr is where it
        // was going anyway. Same for "nobody has taken the screen".
        let Ok(mut b) = buffer().lock() else {
            return io::stderr().write(buf);
        };
        if b.lines.is_none() {
            drop(b);
            return io::stderr().write(buf);
        }
        b.partial.push_str(&String::from_utf8_lossy(buf));
        let mut ready: Vec<String> = Vec::new();
        while let Some(i) = b.partial.find('\n') {
            let line: String = b.partial.drain(..=i).collect();
            let line = strip_ansi(line.trim_end());
            if !line.is_empty() {
                ready.push(line);
            }
        }
        if let Some(held) = b.lines.as_mut() {
            held.extend(ready);
            if held.len() > CAP {
                let over = held.len() - CAP;
                held.drain(..over);
                b.dropped += over;
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Drop the colour `tracing`'s formatter writes.
///
/// The formatter emits SGR escapes, which are right for a terminal reading
/// stderr and wrong for a string about to become a ratatui `Line`: the widget
/// hands the bytes to the terminal *inside* a frame it has already styled, so
/// the codes either fight the frame's own colours or arrive as literal
/// `[33m` garbage. The transcript colours the entry itself, from the level —
/// the same information, expressed the way this surface expresses things.
///
/// Conservative on purpose: a CSI sequence ends at a byte in `@..~`, and
/// anything else beginning `ESC` loses the `ESC` and its successor. There is
/// nothing here worth being clever about, and an unterminated escape reaching
/// a terminal is how a session ends up in a mode nobody chose.
///
/// **Only `ESC`-introduced sequences — every other C0 control passes
/// through, including `\r` and `\n`.** Safe at the call site above because
/// `Writer::write` cuts at `\n` before calling this, so a bare `\r` can never
/// reach it. **Not safe for un-split free text** — see
/// [`strip_ansi_and_controls`], which is.
pub(crate) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // `ESC]…BEL` (OSC) and everything else: drop the pair and move on.
            Some(']') => {
                for c in chars.by_ref() {
                    if c == '\u{7}' {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// [`strip_ansi`] plus every remaining C0/C1 control except tab.
///
/// For a *whole* free-text field rather than a pre-split line — `mecha
/// distill` prints a session's own `Surprise` text (the model's free-text
/// reading of transcript content, which can include a fetched page or a
/// mail body) straight to stdout, and `scripts/ruminate.sh`'s nightly run
/// redirects that output to a dated logfile rather than a live terminal. A
/// bare `\r` in `actual` — the last thing on the printed line — rewrites the
/// rendered line from column 0 in whatever reads the log back, which is
/// enough to erase the very "untrusted, don't act on this" marker the print
/// exists to show; a bare `\n` forges an extra line outright. `strip_ansi`
/// alone does not catch either, because its own call site had already cut
/// the stream at `\n` before this problem could arise. `pub(crate)` rather
/// than a second copy of `strip_ansi`'s ESC-handling loop, on the
/// one-definition rule this codebase keeps paying to relearn.
pub(crate) fn strip_ansi_and_controls(s: &str) -> String {
    strip_ansi(s)
        .chars()
        .filter(|c| *c == '\t' || !c.is_control())
        .collect()
}

/// Installed once in `main`. Every event gets a fresh `Writer`, which is free:
/// the state is the static buffer, not the handle.
#[derive(Clone, Copy)]
pub struct Make;

impl tracing_subscriber::fmt::MakeWriter<'_> for Make {
    type Writer = Writer;
    fn make_writer(&self) -> Writer {
        Writer
    }
}

/// Whether a formatted `tracing` line is one the user should see in red.
///
/// The formatter puts the level first (`without_time`, so nothing precedes
/// it), which is the only thing here that knows about its output shape. An
/// unrecognised line reads as a notice rather than an error, because guessing
/// "alarming" for ordinary debug output is how a colour stops meaning
/// anything.
pub fn is_alarming(line: &str) -> bool {
    let head = line.trim_start();
    head.starts_with("ERROR") || head.starts_with("WARN")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialised because the buffer is a process-wide static, which is what
    /// it has to be — `tracing`'s writer is installed once and called from
    /// every thread in the program.
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static L: OnceLock<Mutex<()>> = OnceLock::new();
        L.get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn nothing_is_held_until_someone_takes_the_screen() {
        let _g = lock();
        let _ = release();
        // Goes to the real stderr, so there is nothing to drain.
        let _ = Writer.write(b"WARN this belongs on the terminal\n");
        assert!(drain().is_empty());
    }

    #[test]
    fn a_line_split_across_writes_is_reassembled() {
        let _g = lock();
        let _ = release();
        capture();
        let _ = Writer.write(b"WARN mecha_core::agent: the run finished on a ");
        // Nothing yet: half a line is not a line.
        assert!(drain().is_empty());
        let _ = Writer.write(b"failed tool call\nDEBUG next\n");
        assert_eq!(
            drain(),
            vec![
                "WARN mecha_core::agent: the run finished on a failed tool call".to_string(),
                "DEBUG next".to_string(),
            ]
        );
        let _ = release();
    }

    #[test]
    fn an_unterminated_line_survives_the_handback() {
        let _g = lock();
        let _ = release();
        capture();
        let _ = Writer.write(b"ERROR half a thought");
        let left = release();
        assert_eq!(left, vec!["ERROR half a thought".to_string()]);
    }

    #[test]
    fn overflow_drops_the_oldest_and_says_how_many() {
        let _g = lock();
        let _ = release();
        capture();
        for i in 0..CAP + 5 {
            let _ = Writer.write(format!("DEBUG line {i}\n").as_bytes());
        }
        let out = drain();
        // The report, then CAP lines, the oldest five gone.
        assert_eq!(out.len(), CAP + 1);
        assert!(out[0].starts_with("5 earlier log line(s) dropped"));
        assert_eq!(out[1], "DEBUG line 5");
        let _ = release();
    }

    #[test]
    fn colour_is_stripped_before_a_line_becomes_a_transcript_entry() {
        // What `tracing_subscriber::fmt` actually emits at warn level.
        let raw = "\u{1b}[33m WARN\u{1b}[0m \u{1b}[2mmecha_core::agent\u{1b}[0m\u{1b}[2m:\u{1b}[0m ended on a failed call";
        assert_eq!(
            strip_ansi(raw),
            " WARN mecha_core::agent: ended on a failed call"
        );
        // And the level still reads through, which is what picks the colour.
        assert!(is_alarming(&strip_ansi(raw)));
    }

    #[test]
    fn an_escape_is_never_left_half_stripped() {
        // A truncated escape reaching a terminal is how a session ends up in
        // a mode nobody chose, so the tail goes with it.
        assert_eq!(strip_ansi("a\u{1b}[31"), "a");
        assert_eq!(strip_ansi("a\u{1b}"), "a");
        assert_eq!(strip_ansi("plain text"), "plain text");
    }

    #[test]
    fn strip_ansi_alone_does_not_catch_a_bare_carriage_return() {
        // The precondition `strip_ansi` relies on — its own call site cuts
        // at `\n` first — does not hold for a whole free-text field, and this
        // is what that gap looks like: a bare `\r` survives and would
        // overwrite everything before it once rendered.
        assert_eq!(strip_ansi("safe\rDANGER"), "safe\rDANGER");
    }

    #[test]
    fn strip_ansi_and_controls_removes_what_strip_ansi_leaves_behind() {
        // A `\r` rewrites the line, a `\n` forges an extra one — both are
        // exactly as effective at defeating a printed warning as the ANSI
        // sequences `strip_ansi` already handles, for a field nothing has
        // pre-split at newlines.
        assert_eq!(strip_ansi_and_controls("safe\rDANGER"), "safeDANGER");
        assert_eq!(
            strip_ansi_and_controls("line one\nline two"),
            "line oneline two"
        );
        assert_eq!(
            strip_ansi_and_controls("a\u{1b}[31mred\u{1b}[0m\rb"),
            "aredb"
        );
        // Tab survives — it is not the threat the others are.
        assert_eq!(strip_ansi_and_controls("a\tb"), "a\tb");
        assert_eq!(strip_ansi_and_controls("plain text"), "plain text");
    }

    #[test]
    fn only_warnings_and_errors_are_alarming() {
        assert!(is_alarming("WARN mecha_core::agent: ..."));
        assert!(is_alarming("  ERROR something"));
        assert!(!is_alarming("DEBUG mecha_core::mcp: connected"));
        assert!(!is_alarming(
            "a bare line from something that is not tracing"
        ));
    }
}
