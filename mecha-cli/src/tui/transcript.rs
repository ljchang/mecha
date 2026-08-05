//! What is on screen, and how it wraps.
//!
//! Kept apart from the event loop because scrolling is the fiddly part: the
//! transcript is a list of *entries*, but scrolling happens in *wrapped screen
//! lines*, and the two only line up once you know how wide the terminal is.

use mecha_core::agent::AgentEvent;
use ratatui::prelude::*;
use ratatui::widgets::{Paragraph, Wrap};

/// One thing that happened, in the order it happened.
///
/// `depth` is how many subagents deep it happened: 0 is the agent the user is
/// talking to, 1 is a tool that is itself an agent, and so on. Rendering
/// indents by it, which is the whole difference between a subagent's work and
/// the parent's.
pub enum Entry {
    User(String),
    /// Typed while the agent was working, so it reads differently: it did not
    /// start this run, it redirected one already underway.
    Steer(String),
    Assistant { text: String, depth: u8 },
    Thinking { text: String, depth: u8 },
    ToolCall { name: String, summary: String, depth: u8 },
    ToolResult { name: String, is_error: bool, content: String, depth: u8 },
    /// The harness talking about itself: what a command did, what was cleared.
    /// May be several lines; each one is rendered as a line, because a
    /// multi-line string pushed as a single `Line` gets reflowed into a blob
    /// the moment the paragraph wraps it.
    Notice(String),
    /// Something went wrong. Separated from `Notice` only so that help text and
    /// a failed run do not arrive in the same alarming colour.
    Error(String),
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
        self.absorb_at(event, 0);
    }

    /// The depth-aware half of [`absorb`]: a `Nested` event unwraps one level
    /// and recurses, so a subagent's turn renders indented under the tool call
    /// that spawned it — and a grandchild's a step further, since it arrives
    /// wrapped twice.
    ///
    /// [`absorb`]: Transcript::absorb
    fn absorb_at(&mut self, event: &AgentEvent, depth: u8) {
        match event {
            AgentEvent::Nested { event, .. } => self.absorb_at(event, depth.saturating_add(1)),
            // A child's prose is detail — its conclusions come back to the
            // parent through the tool result — so it only streams in verbose.
            AgentEvent::TextDelta(t) if depth == 0 || self.verbose => {
                match self.entries.last_mut() {
                    Some(Entry::Assistant { text, depth: d }) if *d == depth => text.push_str(t),
                    _ => self.push(Entry::Assistant { text: t.clone(), depth }),
                }
            }
            AgentEvent::ThinkingDelta(t) if self.verbose => match self.entries.last_mut() {
                Some(Entry::Thinking { text, depth: d }) if *d == depth => text.push_str(t),
                _ => self.push(Entry::Thinking { text: t.clone(), depth }),
            },
            AgentEvent::ToolCall { name, input, .. } => self.push(Entry::ToolCall {
                name: name.clone(),
                summary: crate::approve::summarize(name, input),
                depth,
            }),
            AgentEvent::ToolResult { name, is_error, content, .. } => {
                // Errors are always shown: an agent quietly recovering from a
                // failure is exactly what you want to see.
                if self.verbose || *is_error {
                    self.push(Entry::ToolResult {
                        name: name.clone(),
                        is_error: *is_error,
                        content: content.clone(),
                        depth,
                    });
                }
            }
            AgentEvent::ToolDenied { name, reason } => self.push(Entry::Error(format!(
                "{}{name} denied — {reason}",
                indent(depth)
            ))),
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
                Entry::Assistant { text, depth } => {
                    // A subagent's prose is rendered like detail, not like the
                    // answer: the answer is whatever the parent says about it.
                    let style = if *depth == 0 {
                        Style::new()
                    } else {
                        Style::new().fg(Color::DarkGray)
                    };
                    for line in text.split('\n') {
                        out.push(Line::styled(format!("{}{line}", indent(*depth)), style));
                    }
                }
                Entry::Thinking { text, depth } => {
                    for line in text.split('\n') {
                        out.push(Line::styled(
                            format!("{}{line}", indent(*depth)),
                            Style::new().fg(Color::DarkGray).italic(),
                        ));
                    }
                }
                Entry::ToolCall { name, summary, depth } => {
                    out.push(Line::from(vec![
                        Span::raw(indent(*depth)),
                        Span::styled("● ", Style::new().fg(Color::Magenta)),
                        Span::styled(name.clone(), Style::new().fg(Color::Magenta)),
                        Span::raw(" "),
                        Span::styled(truncate(summary, 200), Style::new().fg(Color::DarkGray)),
                    ]));
                }
                Entry::ToolResult { name, is_error, content, depth } => {
                    let colour = if *is_error { Color::Red } else { Color::DarkGray };
                    out.push(Line::from(vec![
                        Span::raw(indent(*depth)),
                        Span::styled("  ⤷ ", Style::new().fg(colour)),
                        Span::styled(name.clone(), Style::new().fg(colour)),
                        Span::raw(" "),
                        Span::styled(truncate(content, 400), Style::new().fg(colour)),
                    ]));
                }
                Entry::Notice(text) => {
                    for line in text.split('\n') {
                        out.push(Line::styled(line.to_string(), Style::new().fg(Color::DarkGray)));
                    }
                }
                Entry::Error(text) => {
                    for line in text.split('\n') {
                        out.push(Line::styled(line.to_string(), Style::new().fg(Color::Red)));
                    }
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

/// Two spaces per nesting level. A constant width, so a child's rows line up
/// under each other rather than under the varying widths of their markers.
fn indent(depth: u8) -> String {
    "  ".repeat(depth as usize)
}

fn truncate(s: &str, max: usize) -> String {
    let flat = s.replace('\n', " ");
    if flat.chars().count() <= max {
        return flat;
    }
    format!("{}…", flat.chars().take(max).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(name: &str) -> AgentEvent {
        AgentEvent::ToolCall { id: "t1".into(), name: name.into(), input: json!({}) }
    }

    fn nested(tool: &str, event: AgentEvent) -> AgentEvent {
        AgentEvent::Nested { tool: tool.into(), event: Box::new(event) }
    }

    #[test]
    fn a_subagents_call_lands_one_level_deeper_and_a_grandchilds_two() {
        let mut t = Transcript::new(false);
        t.absorb(&call("helper"));
        t.absorb(&nested("helper", call("echo")));
        t.absorb(&nested("helper", nested("inner", call("fs_read"))));

        let depths: Vec<u8> = t
            .entries
            .iter()
            .map(|e| match e {
                Entry::ToolCall { depth, .. } => *depth,
                other => panic!("expected only tool calls, got {}", match other {
                    Entry::User(_) => "user",
                    _ => "something else",
                }),
            })
            .collect();
        assert_eq!(depths, vec![0, 1, 2]);
    }

    #[test]
    fn a_childs_prose_is_detail_and_only_streams_in_verbose() {
        // The child's conclusions come back through the parent's tool result;
        // its running commentary would otherwise be indistinguishable from
        // the answer the user is waiting for.
        let mut quiet = Transcript::new(false);
        quiet.absorb(&nested("helper", AgentEvent::TextDelta("child says".into())));
        assert!(quiet.entries.is_empty(), "hidden while not verbose");

        let mut verbose = Transcript::new(true);
        verbose.absorb(&nested("helper", AgentEvent::TextDelta("child says".into())));
        verbose.absorb(&nested("helper", AgentEvent::TextDelta(" more".into())));
        match &verbose.entries[..] {
            [Entry::Assistant { text, depth: 1 }] => assert_eq!(text, "child says more"),
            other => panic!("expected one folded child entry, got {} entries", other.len()),
        }
    }

    #[test]
    fn child_deltas_do_not_fold_into_the_parents_paragraph() {
        // Same variant, different depth: folding across depths would splice a
        // subagent's words into the middle of the parent's sentence.
        let mut t = Transcript::new(true);
        t.absorb(&AgentEvent::TextDelta("parent".into()));
        t.absorb(&nested("helper", AgentEvent::TextDelta("child".into())));
        t.absorb(&AgentEvent::TextDelta(" continues".into()));

        let texts: Vec<(String, u8)> = t
            .entries
            .iter()
            .filter_map(|e| match e {
                Entry::Assistant { text, depth } => Some((text.clone(), *depth)),
                _ => None,
            })
            .collect();
        assert_eq!(
            texts,
            vec![
                ("parent".into(), 0),
                ("child".into(), 1),
                (" continues".into(), 0),
            ]
        );
    }
}
