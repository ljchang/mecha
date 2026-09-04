//! The /charter modal: the standing priorities every run carries, and the
//! one place a TUI user can edit them without leaving the screen.
//!
//! `mecha charter` is the command-line twin and stays read-only — the
//! charter's write path is *the owner with a text editor*
//! (`docs/GOAL-SYSTEM-DESIGN.md` §11), and this modal does not change that:
//! `e` hands the terminal to `$EDITOR` on `~/.mecha/charter.toml` itself, so
//! mecha never composes a line, and there is still no CLI verb, no tool, and
//! no model path that writes one. The only bytes this surface ever writes
//! are a comments-only template when the file does not exist yet, because
//! `vi` on an empty buffer is how a first charter ends up shaped wrong.
//!
//! Two honesty rules, both learned from siblings:
//!
//! - **A charter that fails to parse is the modal's headline, not a log
//!   line.** `setup` warns on stderr before the TUI takes the screen, so the
//!   alternate screen covers it for the whole session — the /skills rule,
//!   except here the broken document is the one ranking every priority the
//!   agent has, and the run silently started with none.
//! - **An edit reaches the *next* prompt, not this one.** The charter is
//!   rendered into the system prompt once, at agent build — so the modal
//!   says so after every edit rather than letting a saved file read as a
//!   changed conversation. `/model` rebuilds the agent and picks it up.

use mecha_core::charter::{Charter, CharterLine, CHARTER_CHAR_BUDGET};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use std::path::PathBuf;

pub struct CharterModal {
    /// In file order, which is rank order — the whole point of the type.
    pub lines: Vec<CharterLine>,
    pub selected: usize,
    /// Showing one line's full text rather than the list.
    pub detail: bool,
    pub detail_scroll: u16,
    pub path: PathBuf,
    /// The file exists — distinct from `lines.is_empty()`, which is also
    /// true of a template nobody has filled in yet.
    pub exists: bool,
    /// Why the file did not load. `Some` makes the whole modal a failure
    /// report: there is no partial charter, because a document that ranks
    /// priorities cannot drop a line and keep its meaning.
    pub error: Option<String>,
    pub char_count: usize,
    pub over_budget: bool,
    /// One line of feedback after an edit — saved, unchanged, or refused.
    pub status: Option<String>,
    /// Each sensored line's current reading — the detail view shows it
    /// beside the sensor, on §11.1's rule that the editor shows the reading
    /// and the prompt never carries it. **Read on its own thread**, like the
    /// doctor modal's restart probe: `read_charter` is three store reads
    /// and, where a line carries `intervention_rate`, a scan of up to
    /// `doctor::RUNS_WINDOW` transcripts, and `load` runs on the thread that
    /// owns the event loop and the draw — inline, `/charter` froze input
    /// for the length of the read, longest on the machine with the most
    /// history (found on review). `poll` moves the answer in.
    pub readings: Readings,
}

/// The readings' arrival: still on their thread, arrived, or lost with it.
pub enum Readings {
    Pending(std::sync::mpsc::Receiver<Vec<mecha_core::reading::LineReading>>),
    Ready(Vec<mecha_core::reading::LineReading>),
    /// The reader thread ended without answering. Unknown, not empty.
    Lost,
}

impl Readings {
    /// Spawn the read and hand back the receiver.
    fn spawn(charter: Charter) -> Readings {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(mecha_core::reading::read_charter(
                &charter,
                chrono::Utc::now(),
            ));
        });
        Readings::Pending(rx)
    }

    /// The reading for one line, once arrived; `None` while pending or lost.
    fn for_line(&self, id: &str) -> Option<&mecha_core::reading::LineReading> {
        match self {
            Readings::Ready(rs) => rs.iter().find(|r| r.line == id),
            Readings::Pending(_) | Readings::Lost => None,
        }
    }
}

impl CharterModal {
    /// Read the store. Never fails: a parse error becomes the modal's
    /// content, on the rule above.
    pub fn load(path: PathBuf) -> CharterModal {
        let exists = path.is_file();
        match Charter::load(&path) {
            Ok(charter) => CharterModal {
                lines: charter.lines().to_vec(),
                selected: 0,
                detail: false,
                detail_scroll: 0,
                char_count: charter.char_count(),
                over_budget: charter.over_budget(),
                readings: Readings::spawn(charter),
                path,
                exists,
                error: None,
                status: None,
            },
            Err(e) => CharterModal {
                lines: Vec::new(),
                selected: 0,
                detail: false,
                detail_scroll: 0,
                char_count: 0,
                over_budget: false,
                path,
                exists,
                error: Some(format!("{e:#}")),
                status: None,
                readings: Readings::Ready(Vec::new()),
            },
        }
    }

    /// Move an arrived reading in. Called from the loop's tick, never from
    /// `draw`, which is immutable by design.
    pub fn poll(&mut self) {
        let Readings::Pending(rx) = &self.readings else {
            return;
        };
        match rx.try_recv() {
            Ok(readings) => self.readings = Readings::Ready(readings),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.readings = Readings::Lost,
        }
    }

    /// Reload after an edit, keeping the feedback line the caller set.
    pub fn reload(&mut self) {
        let status = self.status.take();
        *self = CharterModal::load(std::mem::take(&mut self.path));
        self.status = status;
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.lines.is_empty() {
            return;
        }
        let len = self.lines.len() as isize;
        self.selected = (((self.selected as isize + delta) % len + len) % len) as usize;
        self.detail_scroll = 0;
    }

    pub fn scroll_detail(&mut self, delta: i16) {
        self.detail_scroll = self.detail_scroll.saturating_add_signed(delta);
    }

    /// Enter opens the full text — only where there is a line to open, on
    /// the /skills rule: a modal that is up must always be on screen.
    pub fn toggle_detail(&mut self) {
        if self.lines.is_empty() {
            return;
        }
        self.detail = !self.detail;
        self.detail_scroll = 0;
    }

    fn title(&self) -> String {
        if self.error.is_some() {
            return " charter failed to load · e edits · esc to close ".into();
        }
        if self.lines.is_empty() {
            // The body makes the same distinction; the title must agree with
            // it or the box describes two different stores at once.
            return if self.exists {
                " charter has no lines yet · e edits · esc to close ".into()
            } else {
                " no charter yet · e creates and edits it · esc to close ".into()
            };
        }
        let budget = if self.over_budget {
            format!(
                " · {} chars, OVER the {CHARTER_CHAR_BUDGET} budget",
                self.char_count
            )
        } else {
            String::new()
        };
        format!(
            " {} standing priorit{}, highest first{budget} · enter for full text · e edits ",
            self.lines.len(),
            if self.lines.len() == 1 { "y" } else { "ies" },
        )
    }

    pub fn draw(&self, frame: &mut Frame) {
        if self.detail {
            self.draw_detail(frame);
        } else {
            self.draw_list(frame);
        }
    }

    fn draw_list(&self, frame: &mut Frame) {
        let mut body: Vec<Line> = Vec::new();

        if let Some(why) = &self.error {
            body.push(Line::styled(
                format!("the charter did not load — {why}"),
                Style::new().fg(Color::Red),
            ));
            body.push(Line::raw(""));
            body.push(Line::styled(
                "every run since has started with NO charter at all: a document that \
                 ranks priorities cannot drop a line and keep its meaning, so a parse \
                 error costs the whole file",
                Style::new().fg(Color::Yellow),
            ));
            body.push(Line::raw(""));
            body.push(Line::styled(
                self.path.display().to_string(),
                Style::new().fg(Color::DarkGray),
            ));
        } else if self.lines.is_empty() {
            body.push(Line::styled(
                if self.exists {
                    "the file exists but has no [[line]] entries — nothing rides in any prompt"
                } else {
                    "no charter has been written yet — nothing rides in any prompt"
                },
                Style::new().fg(Color::White),
            ));
            body.push(Line::raw(""));
            body.push(Line::styled(
                "a charter is a short ranked list of standing priorities, in your own",
                Style::new().fg(Color::White),
            ));
            body.push(Line::styled(
                "words — order is rank: when two conflict, the higher line wins outright",
                Style::new().fg(Color::White),
            ));
            body.push(Line::raw(""));
            body.push(Line::styled(
                "e opens your $EDITOR on the file (with the format explained inside)",
                Style::new().fg(Color::DarkGray),
            ));
        } else {
            for (i, line) in self.lines.iter().enumerate() {
                let text = format!(
                    "{} {}. {:<24} {}",
                    if i == self.selected { "›" } else { " " },
                    i + 1,
                    line.id,
                    line.text.lines().next().unwrap_or(""),
                );
                body.push(if i == self.selected {
                    Line::styled(text, Style::new().fg(Color::Black).bg(Color::Cyan))
                } else {
                    Line::styled(text, Style::new().fg(Color::White))
                });
            }
        }

        if let Some(status) = &self.status {
            body.push(Line::raw(""));
            body.push(Line::styled(status.clone(), Style::new().fg(Color::Yellow)));
        }

        let height = super::list_height(body.len() as u16, frame.area().height);
        let area = super::centered(frame.area(), 100, height);
        frame.render_widget(Clear, area);
        // The ranked list is NOT wrapped, on /skills' own reasoning: the
        // sizing above and `list_scroll` below both count logical entries,
        // and `Paragraph` applies `scroll` after wrapping — so a wrapped
        // list under-scrolls exactly when rows are long, which for a
        // charter ("one or two sentences" per line) is always, and the rows
        // that vanish are the lower-ranked half. A clipped row's full text
        // is one enter away in the detail view. The error and empty bodies
        // have no selection to keep on screen, so they keep the wrap — a
        // parse error must be readable whole.
        let para = Paragraph::new(body)
            .scroll((self.list_scroll(area.height.saturating_sub(2)), 0))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(Color::Cyan))
                    .title(self.title()),
            );
        let para = if self.lines.is_empty() {
            para.wrap(Wrap { trim: false })
        } else {
            para
        };
        frame.render_widget(para, area);
    }

    fn list_scroll(&self, visible: u16) -> u16 {
        let visible = visible.max(1) as usize;
        (self.selected + 1).saturating_sub(visible) as u16
    }

    fn draw_detail(&self, frame: &mut Frame) {
        let Some(line) = self.lines.get(self.selected) else {
            return;
        };
        let mut body: Vec<Line> = vec![
            Line::styled(
                format!(
                    "rank {} of {} — higher wins outright",
                    self.selected + 1,
                    self.lines.len()
                ),
                Style::new().fg(Color::DarkGray),
            ),
            Line::raw(""),
        ];
        for text_line in line.text.lines() {
            body.push(Line::styled(
                text_line.to_string(),
                Style::new().fg(Color::White),
            ));
        }
        // The sensor is shown where the owner reads the line in full, never
        // in the list row and never in a prompt: it is the owner's own
        // setpoint, in their own spelling, so this is the one surface that
        // says a line carries one at all.
        if let Some(sensor) = &line.sensor {
            body.push(Line::raw(""));
            body.push(Line::styled(
                format!(
                    "sensor: {} · setpoint {} — attributes runs that touched what it watches to this line",
                    sensor.kind.wire(),
                    sensor.setpoint_text
                ),
                Style::new().fg(Color::DarkGray),
            ));
            let reading = match &self.readings {
                Readings::Pending(_) => "reading: …".to_string(),
                Readings::Lost => "reading: lost — `mecha charter` reads it".to_string(),
                Readings::Ready(_) => match self.readings.for_line(&line.id) {
                    Some(r) => format!("reading: {}", r.summary()),
                    None => "reading: none".to_string(),
                },
            };
            body.push(Line::styled(reading, Style::new().fg(Color::DarkGray)));
        }
        let area = super::centered(
            frame.area(),
            84,
            (body.len() as u16 + 4).min(frame.area().height),
        );
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(body)
                .wrap(Wrap { trim: false })
                .scroll((self.detail_scroll, 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::new().fg(Color::Cyan))
                        .title(format!(" {} · ↑↓ scrolls · esc to go back ", line.id)),
                ),
            area,
        );
    }
}

// The template re-export used to live here, for `mod.rs`'s own editor
// hand-over. Both the write and the did-anything-land classification are
// `editor::edit_charter_with`'s now — one implementation shared with
// `mecha charter edit` — so this modal no longer touches the constant, and a
// re-export nothing reads is a second path to a thing that has one.

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, text: &str) -> CharterLine {
        CharterLine {
            id: id.into(),
            text: text.into(),
            sensor: None,
        }
    }

    fn modal(lines: Vec<CharterLine>) -> CharterModal {
        CharterModal {
            lines,
            selected: 0,
            detail: false,
            detail_scroll: 0,
            path: PathBuf::from("/nowhere/charter.toml"),
            exists: true,
            error: None,
            char_count: 100,
            over_budget: false,
            status: None,
            readings: Readings::Ready(Vec::new()),
        }
    }

    /// The reading arrives on its thread and `poll` moves it in; a thread
    /// that dies is a lost reading, never an empty one.
    #[test]
    fn readings_arrive_through_poll_and_a_dead_reader_reads_as_lost() {
        let mut m = modal(vec![line("waits", "Keep it short.")]);
        let (tx, rx) = std::sync::mpsc::channel();
        m.readings = Readings::Pending(rx);
        m.poll();
        assert!(matches!(m.readings, Readings::Pending(_)));
        tx.send(vec![mecha_core::reading::LineReading {
            line: "waits".into(),
            kind: mecha_core::charter::SensorKind::OutboxAge,
            setpoint: "24h".into(),
            reading: mecha_core::reading::Reading::Nothing,
        }])
        .unwrap();
        m.poll();
        assert_eq!(
            m.readings.for_line("waits").map(|r| r.summary()),
            Some("nothing waiting".to_string())
        );

        let (tx, rx) = std::sync::mpsc::channel::<Vec<mecha_core::reading::LineReading>>();
        m.readings = Readings::Pending(rx);
        drop(tx);
        m.poll();
        assert!(matches!(m.readings, Readings::Lost));
        assert!(m.readings.for_line("waits").is_none());
    }

    #[test]
    fn a_parse_failure_is_the_headline_not_a_row() {
        let dir = std::env::temp_dir().join(format!("mecha-charter-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("charter.toml");
        std::fs::write(&p, "[[line]]\nid = \"a\"\n").unwrap(); // missing text
        let m = CharterModal::load(p);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(m.error.is_some());
        assert!(m.lines.is_empty());
        assert!(m.title().contains("failed to load"), "{}", m.title());
    }

    #[test]
    fn a_missing_file_and_an_empty_file_read_differently() {
        let m = CharterModal::load(PathBuf::from("/nowhere/does-not-exist.toml"));
        assert!(m.error.is_none(), "missing is empty, not broken");
        assert!(!m.exists);
        assert!(m.title().contains("no charter yet"), "{}", m.title());
    }

    #[test]
    fn the_title_counts_lines_and_flags_the_budget() {
        let m = modal(vec![line("a", "first"), line("b", "second")]);
        assert!(m.title().contains("2 standing priorities"), "{}", m.title());
        assert!(!m.title().contains("OVER"));

        let over = CharterModal {
            over_budget: true,
            char_count: 2500,
            ..modal(vec![line("a", "first")])
        };
        assert!(over.title().contains("OVER"), "{}", over.title());
        assert!(
            over.title().contains("1 standing priority,"),
            "{}",
            over.title()
        );
    }

    /// The /skills rule: enter on an empty store must not flip into a detail
    /// view that renders nothing while still owning the keyboard.
    #[test]
    fn an_empty_charter_cannot_be_toggled_into_an_invisible_detail() {
        let mut m = modal(Vec::new());
        m.toggle_detail();
        assert!(!m.detail);
    }

    #[test]
    fn the_selection_wraps_and_moving_resets_the_detail_scroll() {
        let mut m = modal(vec![line("a", "one"), line("b", "two")]);
        m.move_by(-1);
        assert_eq!(m.selected, 1);
        m.scroll_detail(7);
        m.move_by(1);
        assert_eq!((m.selected, m.detail_scroll), (0, 0));
    }

    /// The shared-`list_height` rule: a four-row terminal shrinks the box
    /// rather than panicking on `clamp(min > max)`.
    #[test]
    fn a_tiny_terminal_shrinks_the_list_rather_than_panicking() {
        use ratatui::Terminal;
        let m = modal(vec![line("a", "one"), line("b", "two"), line("c", "three")]);
        for height in 0..=6u16 {
            let mut terminal =
                Terminal::new(ratatui::backend::TestBackend::new(100, height.max(1))).unwrap();
            terminal.draw(|f| m.draw(f)).unwrap();
        }
    }

    /// The /skills rule the first cut broke: the ranked list must not be
    /// wrapped, because `list_height` and `list_scroll` both count logical
    /// entries while `Paragraph` scrolls after wrapping — so long lines
    /// (a charter's normal case) made low-ranked rows unreachable: the
    /// selection walked off screen with nothing indicating it moved.
    #[test]
    fn a_long_lined_charter_keeps_the_selection_on_screen() {
        use ratatui::Terminal;
        let long = "a standing priority long enough to wrap several times in any \
                    reasonable terminal, which is the normal case for a charter \
                    line of one or two sentences rather than a slug";
        let m = CharterModal {
            selected: 7,
            ..modal((0..8).map(|i| line(&format!("p{i}"), long)).collect())
        };
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(80, 10)).unwrap();
        terminal.draw(|f| m.draw(f)).unwrap();
        let shown = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(
            shown.contains("p7"),
            "the selected (last-ranked) row is off screen"
        );
    }

    /// Reload keeps the feedback line: it is set by the editor round-trip
    /// and the reload happens immediately after, so dropping it would blank
    /// the one message the keystroke existed to produce.
    #[test]
    fn reload_keeps_the_status_line() {
        let mut m = CharterModal::load(PathBuf::from("/nowhere/does-not-exist.toml"));
        m.status = Some("saved".into());
        m.reload();
        assert_eq!(m.status.as_deref(), Some("saved"));
    }
}
