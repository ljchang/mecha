//! What is on screen, and how it wraps.
//!
//! Kept apart from the event loop because scrolling is the fiddly part: the
//! transcript is a list of *entries*, but scrolling happens in *wrapped screen
//! lines*, and the two only line up once you know how wide the terminal is.

use mecha_core::agent::AgentEvent;
use ratatui::prelude::*;
use ratatui::widgets::{Paragraph, Wrap};

/// One thing that happened, in the order it happened.
pub enum Entry {
    User(String),
    /// Typed while the agent was working, so it reads differently: it did not
    /// start this run, it redirected one already underway.
    Steer(String),
    Assistant(String),
    Thinking(String),
    ToolCall { name: String, summary: String },
    ToolResult { name: String, is_error: bool, content: String },
    /// The harness talking about itself: denials, interruptions, errors.
    Notice(String),
}

#[derive(Default)]
pub struct Transcript {
    entries: Vec<Entry>,
    /// Screen lines scrolled past. Grows downward, 0 is the top.
    pub scroll: u16,
    /// Stick to the bottom as new output arrives — until the user scrolls up,
    /// at which point staying put is the only sane behaviour.
    pub follow: bool,
    /// Show thinking and tool output.
    pub verbose: bool,
}

impl Transcript {
    pub fn new(verbose: bool) -> Self {
        Transcript { follow: true, verbose, ..Default::default() }
    }

    pub fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
    }

    /// Fold streaming output into the entry it belongs to, so a turn arriving
    /// as two hundred deltas is one paragraph rather than two hundred.
    pub fn absorb(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::TextDelta(t) => match self.entries.last_mut() {
                Some(Entry::Assistant(text)) => text.push_str(t),
                _ => self.push(Entry::Assistant(t.clone())),
            },
            AgentEvent::ThinkingDelta(t) if self.verbose => match self.entries.last_mut() {
                Some(Entry::Thinking(text)) => text.push_str(t),
                _ => self.push(Entry::Thinking(t.clone())),
            },
            AgentEvent::ToolCall { name, input, .. } => self.push(Entry::ToolCall {
                name: name.clone(),
                summary: crate::approve::summarize(name, input),
            }),
            AgentEvent::ToolResult { name, is_error, content, .. } => {
                // Errors are always shown: an agent quietly recovering from a
                // failure is exactly what you want to see.
                if self.verbose || *is_error {
                    self.push(Entry::ToolResult {
                        name: name.clone(),
                        is_error: *is_error,
                        content: content.clone(),
                    });
                }
            }
            AgentEvent::ToolDenied { name, reason } => {
                self.push(Entry::Notice(format!("{name} denied — {reason}")))
            }
            AgentEvent::QueuedInput(text) => self.push(Entry::Steer(text.clone())),
            _ => {}
        }
    }

    /// Render to styled lines. Rebuilt every frame, which is cheap next to the
    /// terminal write and keeps wrapping honest when the window is resized.
    ///
    /// Owned rather than borrowed from `self`, so `draw` can still adjust the
    /// scroll after measuring the result.
    fn lines(&self) -> Vec<Line<'static>> {
        let mut out: Vec<Line<'static>> = Vec::new();

        for entry in &self.entries {
            match entry {
                Entry::User(text) => {
                    out.push(Line::from(vec![
                        Span::styled("› ", Style::new().fg(Color::Cyan).bold()),
                        Span::styled(text.clone(), Style::new().bold()),
                    ]));
                }
                Entry::Steer(text) => {
                    out.push(Line::from(vec![
                        Span::styled("↳ ", Style::new().fg(Color::Yellow).bold()),
                        Span::styled(text.clone(), Style::new().fg(Color::Yellow)),
                        Span::styled("  (steering)", Style::new().fg(Color::DarkGray)),
                    ]));
                }
                Entry::Assistant(text) => {
                    for line in text.split('\n') {
                        out.push(Line::raw(line.to_string()));
                    }
                }
                Entry::Thinking(text) => {
                    for line in text.split('\n') {
                        out.push(Line::styled(line.to_string(), Style::new().fg(Color::DarkGray).italic()));
                    }
                }
                Entry::ToolCall { name, summary } => {
                    out.push(Line::from(vec![
                        Span::styled("● ", Style::new().fg(Color::Magenta)),
                        Span::styled(name.clone(), Style::new().fg(Color::Magenta)),
                        Span::raw(" "),
                        Span::styled(truncate(summary, 200), Style::new().fg(Color::DarkGray)),
                    ]));
                }
                Entry::ToolResult { name, is_error, content } => {
                    let colour = if *is_error { Color::Red } else { Color::DarkGray };
                    out.push(Line::from(vec![
                        Span::styled("  ⤷ ", Style::new().fg(colour)),
                        Span::styled(name.clone(), Style::new().fg(colour)),
                        Span::raw(" "),
                        Span::styled(truncate(content, 400), Style::new().fg(colour)),
                    ]));
                }
                Entry::Notice(text) => {
                    out.push(Line::styled(text.clone(), Style::new().fg(Color::Red)));
                }
            }
        }
        out
    }

    /// Draw into `area`, honouring follow-mode and clamping the scroll so the
    /// view can never end up past the end of the content.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let lines = self.lines();
        let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });

        // Wrapped height, which is what scrolling is measured in — not the
        // number of entries, and not the number of unwrapped lines.
        let height = paragraph.line_count(area.width) as u16;
        let max_scroll = height.saturating_sub(area.height);

        if self.follow {
            self.scroll = max_scroll;
        } else {
            self.scroll = self.scroll.min(max_scroll);
            // Scrolling back to the bottom re-arms follow mode, so the user
            // doesn't have to know it exists.
            if self.scroll == max_scroll {
                self.follow = true;
            }
        }

        frame.render_widget(paragraph.scroll((self.scroll, 0)), area);
    }

    pub fn scroll_up(&mut self, lines: u16) {
        self.follow = false;
        self.scroll = self.scroll.saturating_sub(lines);
    }

    pub fn scroll_down(&mut self, lines: u16) {
        // Clamped against the real maximum at draw time, where the width is
        // known; overshooting here is harmless.
        self.scroll = self.scroll.saturating_add(lines);
    }

    pub fn jump_to_bottom(&mut self) {
        self.follow = true;
    }
}

fn truncate(s: &str, max: usize) -> String {
    let flat = s.replace('\n', " ");
    if flat.chars().count() <= max {
        return flat;
    }
    format!("{}…", flat.chars().take(max).collect::<String>())
}
