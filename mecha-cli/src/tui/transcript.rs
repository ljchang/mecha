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
    /// A `!command` the user ran themselves. Local to the session: the model
    /// never sees it, no taint, no approval — it is the user's own terminal.
    Shell { cmd: String, output: String, status: Option<i32> },
    /// The harness talking about itself: what a command did, what was cleared.
    /// May be several lines; each one is rendered as a line, because a
    /// multi-line string pushed as a single `Line` gets reflowed into a blob
    /// the moment the paragraph wraps it.
    Notice(String),
    /// Something went wrong. Separated from `Notice` only so that help text and
    /// a failed run do not arrive in the same alarming colour.
    Error(String),
}

/// One entry's cached render: the styled lines and their wrapped height, valid
/// for exactly one `(width, verbose)` pair. `key: None` means stale.
///
/// The codex insight, ported: committed history is immutable, so it renders
/// once. Without this, every streamed token re-styled and re-wrapped the
/// whole transcript — O(session) per frame, twice (once to measure, once to
/// paint), which is what made the input line go sticky in exactly the
/// long-horizon runs this harness is built for.
#[derive(Default)]
struct Cell {
    key: Option<(u16, bool)>,
    lines: Vec<Line<'static>>,
    height: usize,
}

#[derive(Default)]
pub struct Transcript {
    entries: Vec<Entry>,
    /// Parallel to `entries`. Kept in lockstep by `push`; the only mutation
    /// that isn't a push is a delta folding into the tail, which invalidates
    /// the tail's cell. Width and verbose changes need no bookkeeping at all:
    /// they change the key, and a mismatched key rebuilds lazily.
    cache: Vec<Cell>,
    /// Screen lines scrolled past. Grows downward, 0 is the top.
    pub scroll: u16,
    /// Stick to the bottom as new output arrives — until the user scrolls up,
    /// at which point staying put is the only sane behaviour.
    pub follow: bool,
    /// Show thinking and tool output. A *render* filter, not a record filter:
    /// everything is kept, so toggling mid-session (^O) reveals what already
    /// happened rather than only what happens next. `--verbose` is just the
    /// initial position.
    pub verbose: bool,
}

impl Transcript {
    pub fn new(verbose: bool) -> Self {
        Transcript { follow: true, verbose, ..Default::default() }
    }

    pub fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
        self.cache.push(Cell::default());
    }

    /// The tail entry changed in place (a delta folded in): its cached render
    /// is now a lie.
    fn invalidate_last(&mut self) {
        if let Some(cell) = self.cache.last_mut() {
            cell.key = None;
        }
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
            AgentEvent::TextDelta(t) => {
                let folded = match self.entries.last_mut() {
                    Some(Entry::Assistant { text, depth: d }) if *d == depth => {
                        text.push_str(t);
                        true
                    }
                    _ => false,
                };
                if folded {
                    self.invalidate_last();
                } else {
                    self.push(Entry::Assistant { text: t.clone(), depth });
                }
            }
            AgentEvent::ThinkingDelta(t) => {
                let folded = match self.entries.last_mut() {
                    Some(Entry::Thinking { text, depth: d }) if *d == depth => {
                        text.push_str(t);
                        true
                    }
                    _ => false,
                };
                if folded {
                    self.invalidate_last();
                } else {
                    self.push(Entry::Thinking { text: t.clone(), depth });
                }
            }
            AgentEvent::ToolCall { name, input, .. } => self.push(Entry::ToolCall {
                name: name.clone(),
                summary: crate::approve::summarize(name, input),
                depth,
            }),
            AgentEvent::ToolResult { name, is_error, content, .. } => {
                self.push(Entry::ToolResult {
                    name: name.clone(),
                    is_error: *is_error,
                    content: content.clone(),
                    depth,
                });
            }
            AgentEvent::ToolDenied { name, reason } => self.push(Entry::Error(format!(
                "{}{name} denied — {reason}",
                indent(depth)
            ))),
            AgentEvent::QueuedInput(text) => self.push(Entry::Steer(text.clone())),
            _ => {}
        }
    }

    /// What `verbose` decides, in one place: detail entries render only when
    /// it is on. Errors always render — an agent quietly recovering from a
    /// failure is exactly what you want to see — and so does the parent's own
    /// prose, which is the answer being waited on.
    #[cfg(test)]
    fn visible(&self, entry: &Entry) -> bool {
        visible(entry, self.verbose)
    }

    /// Every visible entry rendered to styled lines, uncached — the reference
    /// implementation the cell cache must agree with, kept for exactly the
    /// tests that say so.
    #[cfg(test)]
    fn lines(&self) -> Vec<Line<'static>> {
        self.entries
            .iter()
            .filter(|e| self.visible(e))
            .flat_map(entry_lines)
            .collect()
    }
}

/// What `verbose` decides, in one place: see [`Transcript::visible`].
fn visible(entry: &Entry, verbose: bool) -> bool {
    match entry {
        Entry::Thinking { .. } => verbose,
        Entry::ToolResult { is_error, .. } => verbose || *is_error,
        // A child's prose is detail — its conclusions come back to the
        // parent through the tool result.
        Entry::Assistant { depth, .. } => *depth == 0 || verbose,
        _ => true,
    }
}

/// One entry's styled lines. A free function so the cache refresh can hold
/// `&mut` cells while rendering the entries beside them.
fn entry_lines(entry: &Entry) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    {
        {
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
                Entry::Shell { cmd, output, status } => {
                    // Non-zero exit colours the header, not the output: the
                    // output of a failed command is still just output.
                    let header_colour = match status {
                        Some(0) => Color::Cyan,
                        _ => Color::Red,
                    };
                    let exit = match status {
                        Some(0) => String::new(),
                        Some(code) => format!("  (exit {code})"),
                        None => "  (killed)".to_string(),
                    };
                    out.push(Line::from(vec![
                        Span::styled("! ", Style::new().fg(header_colour).bold()),
                        Span::styled(cmd.clone(), Style::new().fg(header_colour)),
                        Span::styled(exit, Style::new().fg(Color::DarkGray)),
                    ]));
                    for line in output.lines() {
                        out.push(Line::styled(
                            format!("  {line}"),
                            Style::new().fg(Color::DarkGray),
                        ));
                    }
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
    }
    out
}

impl Transcript {
    /// Draw into `area`, honouring follow-mode and clamping the scroll so the
    /// view can never end up past the end of the content.
    ///
    /// Served from the cell cache: an ordinary streaming frame re-renders
    /// only the tail entry, and only the cells intersecting the viewport are
    /// cloned into the paragraph. A resize or a ^O toggle changes the cache
    /// key and rebuilds everything exactly once. Safe to slice by cells
    /// because ratatui wraps each `Line` independently — a wrap never crosses
    /// an entry boundary.
    pub fn draw(&mut self, frame: &mut Frame, area: Rect) {
        let key = (area.width, self.verbose);

        // Refresh stale cells and take the running total — the whole-height
        // measurement `line_count` used to redo per frame.
        let mut total: usize = 0;
        for (entry, cell) in self.entries.iter().zip(self.cache.iter_mut()) {
            if cell.key != Some(key) {
                cell.lines = if visible(entry, self.verbose) {
                    entry_lines(entry)
                } else {
                    Vec::new()
                };
                cell.height = wrapped_height(&cell.lines, area.width);
                cell.key = Some(key);
            }
            total += cell.height;
        }

        // Wrapped height, which is what scrolling is measured in — not the
        // number of entries, and not the number of unwrapped lines.
        let height = total.min(u16::MAX as usize) as u16;
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

        // Only the cells intersecting the viewport, with the scroll made
        // local to the first of them.
        let scroll = self.scroll as usize;
        let end_wanted = scroll + area.height as usize;
        let mut acc = 0usize;
        let mut local_scroll = 0u16;
        let mut started = false;
        let mut lines: Vec<Line<'static>> = Vec::new();
        for cell in &self.cache {
            if cell.height == 0 {
                continue;
            }
            let end = acc + cell.height;
            if end > scroll && acc < end_wanted {
                if !started {
                    local_scroll = (scroll - acc) as u16;
                    started = true;
                }
                lines.extend(cell.lines.iter().cloned());
            }
            acc = end;
            if acc >= end_wanted {
                break;
            }
        }

        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((local_scroll, 0)),
            area,
        );
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

/// How many screen rows these lines occupy at `width`, wrapped the same way
/// the paragraph will wrap them. Paid once per cell rebuild, not per frame.
fn wrapped_height(lines: &[Line<'static>], width: u16) -> usize {
    if lines.is_empty() {
        return 0;
    }
    Paragraph::new(lines.to_vec()).wrap(Wrap { trim: false }).line_count(width)
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

    /// Everything a transcript would render, as one string, for asserting on
    /// what is *visible* rather than what is recorded.
    fn rendered(t: &Transcript) -> String {
        t.lines()
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_childs_prose_is_detail_and_only_renders_in_verbose() {
        // The child's conclusions come back through the parent's tool result;
        // its running commentary would otherwise be indistinguishable from
        // the answer the user is waiting for. Recorded either way — the
        // toggle has to be able to reveal it later.
        let mut t = Transcript::new(false);
        t.absorb(&nested("helper", AgentEvent::TextDelta("child says".into())));
        t.absorb(&nested("helper", AgentEvent::TextDelta(" more".into())));

        assert!(!t.entries.is_empty(), "recorded even while hidden");
        assert!(!rendered(&t).contains("child says"), "hidden while not verbose");

        t.verbose = true;
        assert!(rendered(&t).contains("child says more"), "revealed by the toggle, folded");
    }

    /// Render `t` through the cell cache (the real `draw`) into a buffer.
    fn cached_frame(t: &mut Transcript, width: u16, height: u16) -> ratatui::buffer::Buffer {
        use ratatui::backend::TestBackend;
        let mut terminal = ratatui::Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| t.draw(frame, frame.area())).unwrap();
        terminal.backend().buffer().clone()
    }

    /// Render `t` the pre-cache way — one paragraph of every visible line,
    /// scrolled to wherever `draw` left `t.scroll` — into a buffer.
    fn reference_frame(t: &Transcript, width: u16, height: u16) -> ratatui::buffer::Buffer {
        use ratatui::backend::TestBackend;
        let mut terminal = ratatui::Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(
                    Paragraph::new(t.lines())
                        .wrap(Wrap { trim: false })
                        .scroll((t.scroll, 0)),
                    frame.area(),
                );
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// A transcript with every entry kind, long text that wraps, and nesting.
    fn busy_transcript(entries: usize) -> Transcript {
        let mut t = Transcript::new(false);
        for i in 0..entries {
            t.push(Entry::User(format!("question {i}, padded so that it wraps at any sane width {}", "x".repeat(40))));
            t.absorb(&call("fs_read"));
            t.absorb(&AgentEvent::ToolResult {
                id: format!("t{i}"),
                name: "fs_read".into(),
                is_error: i % 7 == 0,
                content: format!("result {i} {}", "y".repeat(120)),
            });
            t.absorb(&nested("helper", call("echo")));
            t.absorb(&AgentEvent::TextDelta(format!("answer {i} ")));
            t.absorb(&AgentEvent::TextDelta("and the rest of it\n".into()));
        }
        t
    }

    #[test]
    fn the_cached_frame_is_identical_to_the_uncached_one() {
        // The whole risk of a render cache is a stale cell surviving a
        // mutation. Every invalidation path in one test: initial render,
        // a delta folding into the tail, a fresh entry, a resize (key
        // change), the verbose toggle (key change), and a scroll.
        let mut t = busy_transcript(30);

        assert_eq!(cached_frame(&mut t, 80, 20), reference_frame(&t, 80, 20), "initial");

        t.absorb(&AgentEvent::TextDelta("more streamed text".into()));
        assert_eq!(cached_frame(&mut t, 80, 20), reference_frame(&t, 80, 20), "tail mutated");

        t.push(Entry::Notice("a new entry".into()));
        assert_eq!(cached_frame(&mut t, 80, 20), reference_frame(&t, 80, 20), "entry added");

        assert_eq!(cached_frame(&mut t, 120, 20), reference_frame(&t, 120, 20), "resized wider");
        assert_eq!(cached_frame(&mut t, 41, 20), reference_frame(&t, 41, 20), "resized narrower");

        t.verbose = true;
        assert_eq!(cached_frame(&mut t, 80, 20), reference_frame(&t, 80, 20), "verbose on");
        t.verbose = false;
        assert_eq!(cached_frame(&mut t, 80, 20), reference_frame(&t, 80, 20), "verbose off again");

        t.scroll_up(17);
        assert_eq!(cached_frame(&mut t, 80, 20), reference_frame(&t, 80, 20), "scrolled back");
        t.scroll_up(9999);
        assert_eq!(cached_frame(&mut t, 80, 20), reference_frame(&t, 80, 20), "clamped at the top");
        t.jump_to_bottom();
        assert_eq!(cached_frame(&mut t, 80, 20), reference_frame(&t, 80, 20), "followed again");
    }

    #[test]
    #[ignore = "a measurement, not a regression test — run with --ignored --nocapture"]
    fn frame_time_before_and_after_the_cache() {
        let mut t = busy_transcript(100); // ~500 entries
        let (width, height) = (130u16, 45u16);

        // Warm the cache once, then measure steady-state streaming frames:
        // one delta folds into the tail per frame, which is the real workload.
        cached_frame(&mut t, width, height);
        let start = std::time::Instant::now();
        for _ in 0..100 {
            t.absorb(&AgentEvent::TextDelta("token ".into()));
            cached_frame(&mut t, width, height);
        }
        let cached = start.elapsed();

        let start = std::time::Instant::now();
        for _ in 0..100 {
            t.absorb(&AgentEvent::TextDelta("token ".into()));
            // The pre-cache pipeline: rebuild and re-wrap everything.
            let lines = t.lines();
            let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
            let _height = paragraph.line_count(width);
            reference_frame(&t, width, height);
        }
        let uncached = start.elapsed();

        println!(
            "100 streaming frames over ~{} entries: cached {cached:?}, uncached {uncached:?} ({}x)",
            t.entries.len(),
            (uncached.as_nanos() / cached.as_nanos().max(1))
        );
    }

    #[test]
    fn the_verbose_toggle_reveals_tool_output_that_already_happened() {
        // The old behaviour dropped hidden entries at absorb time, which made
        // a later toggle only apply to the future — precisely backwards, since
        // the moment you reach for ^O is after something looked wrong.
        let mut t = Transcript::new(false);
        t.absorb(&call("fs_read"));
        t.absorb(&AgentEvent::ToolResult {
            id: "t1".into(),
            name: "fs_read".into(),
            is_error: false,
            content: "the file contents".into(),
        });

        assert!(!rendered(&t).contains("the file contents"));
        t.verbose = true;
        assert!(rendered(&t).contains("the file contents"));

        // Errors were never hidden, toggle or no toggle.
        t.verbose = false;
        t.absorb(&AgentEvent::ToolResult {
            id: "t2".into(),
            name: "shell".into(),
            is_error: true,
            content: "exit 1: no such file".into(),
        });
        assert!(rendered(&t).contains("no such file"));
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
