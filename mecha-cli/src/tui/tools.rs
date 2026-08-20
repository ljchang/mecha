//! The /tools modal: what the agent can call, and what each tool declares.
//!
//! A modal rather than a transcript dump because the useful question has two
//! depths: "what is here" is a glance down a list, and "what does this one
//! actually do" — the full description, the declared capabilities, whether
//! calls are staged — is a detail view you open on the one you mean. Dumping
//! all of that for every tool made /tools unreadable at exactly the tool
//! counts where it matters.

use mecha_core::tool::Capabilities;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

pub struct ToolRow {
    pub name: String,
    pub read_only: bool,
    /// Calls are staged in the outbox for review instead of executed.
    pub outbox: bool,
    pub caps: Capabilities,
    pub description: String,
}

impl ToolRow {
    /// The compact risk summary shown beside the name. Order is by how much
    /// the user should care, and absence is information: no badges means the
    /// tool declared nothing special.
    fn badges(&self) -> String {
        let mut parts: Vec<&str> = vec![if self.read_only { "ro" } else { "writes" }];
        if self.outbox {
            parts.push("outbox");
        }
        if self.caps.external_send {
            parts.push("sends");
        }
        if self.caps.untrusted_input {
            parts.push("untrusted");
        }
        if self.caps.private_data {
            parts.push("private");
        }
        if self.caps.destructive {
            parts.push("destructive");
        }
        parts.join(" · ")
    }
}

pub struct ToolsModal {
    pub rows: Vec<ToolRow>,
    pub selected: usize,
    /// Showing the detail view for the selected row rather than the list.
    pub detail: bool,
    /// How far the detail view is scrolled.
    ///
    /// The detail's body is a **tool description**, and for an MCP tool that
    /// is text a third-party server wrote — unbounded, exactly like a
    /// `SKILL.md`, which is why `/skills` grew this first. Without it the
    /// declared-capability block, which is the entire reason to open this
    /// view, sits below the fold on any real MCP tool with no key that
    /// reaches it: measured at 22 rows, a 25-line description cut at line 18
    /// and "reads data the user considers private" was simply not on screen.
    ///
    /// Reset on every move, like the sibling modals': an offset carried onto
    /// another row is a position in a different document.
    pub detail_scroll: u16,
    /// What `shell` actually is right now, shown on its detail view — the
    /// sandbox decides, and this modal is where the user looks.
    pub sandbox_line: String,
}

impl ToolsModal {
    pub fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let len = self.rows.len() as isize;
        // Wraps, like the picker: a list is faster to cycle than to bound.
        self.selected = (((self.selected as isize + delta) % len + len) % len) as usize;
        self.detail_scroll = 0;
    }

    pub fn scroll_detail(&mut self, delta: i16) {
        self.detail_scroll = self.detail_scroll.saturating_add_signed(delta);
    }

    pub fn draw(&self, frame: &mut Frame) {
        if self.detail {
            self.draw_detail(frame);
        } else {
            self.draw_list(frame);
        }
    }

    fn draw_list(&self, frame: &mut Frame) {
        let body: Vec<Line> = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let text = format!(
                    "{} {:<22} [{}]  {}",
                    if i == self.selected { "›" } else { " " },
                    row.name,
                    row.badges(),
                    row.description.lines().next().unwrap_or(""),
                );
                if i == self.selected {
                    Line::styled(text, Style::new().fg(Color::Black).bg(Color::Cyan))
                } else {
                    Line::styled(text, Style::new().fg(Color::White))
                }
            })
            .collect();

        let height = super::list_height(body.len() as u16, frame.area().height);
        let area = super::centered(frame.area(), 100, height);
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(body)
                .scroll((self.list_scroll(area.height.saturating_sub(2)), 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::new().fg(Color::Cyan))
                        .title(format!(
                            " {} tools · enter for detail, esc to close ",
                            self.rows.len()
                        )),
                ),
            area,
        );
    }

    /// Keep the selection on screen when the list is taller than the modal.
    fn list_scroll(&self, visible: u16) -> u16 {
        let visible = visible.max(1) as usize;
        (self.selected + 1).saturating_sub(visible) as u16
    }

    fn draw_detail(&self, frame: &mut Frame) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };

        let mut body: Vec<Line> = vec![Line::styled(
            format!("[{}]", row.badges()),
            Style::new().fg(Color::DarkGray),
        )];
        body.push(Line::raw(""));
        for line in row.description.lines() {
            body.push(Line::styled(
                line.to_string(),
                Style::new().fg(Color::White),
            ));
        }
        body.push(Line::raw(""));

        // The declared risk surface, spelled out. Only what is set: absence
        // reads better as one line than as four noes.
        let mut declared: Vec<&str> = Vec::new();
        if row.caps.private_data {
            declared.push("reads data the user considers private");
        }
        if row.caps.untrusted_input {
            declared.push("returns content a third party can influence");
        }
        if row.caps.external_send {
            declared.push("can transmit data outside the user's control");
        }
        if row.caps.destructive {
            declared.push("may destroy or overwrite data");
        }
        if declared.is_empty() {
            body.push(Line::styled(
                "declares no special risk surface",
                Style::new().fg(Color::DarkGray),
            ));
        } else {
            for line in declared {
                body.push(Line::styled(
                    format!("• {line}"),
                    Style::new().fg(Color::Yellow),
                ));
            }
        }
        if row.outbox {
            body.push(Line::styled(
                "• calls are staged in the outbox for your review, never sent directly",
                Style::new().fg(Color::Green),
            ));
        }
        if row.name == "shell" {
            body.push(Line::styled(
                format!("• {}", self.sandbox_line),
                Style::new().fg(Color::Yellow),
            ));
        }

        let area = super::centered(
            frame.area(),
            80,
            (body.len() as u16 + 4).min(frame.area().height),
        );
        frame.render_widget(Clear, area);
        // `Wrap` means the drawn height is not `body.len()`, so a body that
        // looks shorter than the box can still lose its tail — the bound is
        // measured rather than assumed, the same way the transcript measures
        // its own scrollable height.
        let paragraph = Paragraph::new(body).wrap(Wrap { trim: false });
        let inner = area.width.saturating_sub(2);
        let drawn = paragraph.line_count(inner) as u16;
        let visible = area.height.saturating_sub(2);
        let max_scroll = drawn.saturating_sub(visible);
        let scroll = self.detail_scroll.min(max_scroll);
        let title = if max_scroll == 0 {
            format!(" {} · esc to go back ", row.name)
        } else {
            format!(
                " {} · {}/{} · ↑↓ scrolls · esc to go back ",
                row.name,
                (scroll + visible).min(drawn),
                drawn
            )
        };
        frame.render_widget(
            paragraph.scroll((scroll, 0)).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(Color::Cyan))
                    .title(title),
            ),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ratatui::backend::TestBackend;

    fn frame_text(modal: &ToolsModal, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| modal.draw(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The detail's body is whatever an MCP server wrote in its tool
    /// description, so it has no length bound at all — and the block that says
    /// what the tool may do to you is at the *bottom* of it. Measured before
    /// the fix: at 22 rows a 25-line description cut at line 18 and
    /// "reads data the user considers private" was simply not on screen, with
    /// no key that could reach it (`Up`/`Down` were guarded `if !detail`).
    #[test]
    fn a_long_description_does_not_bury_the_capability_block_unreachably() {
        let long: String = (1..=25)
            .map(|i| format!("description line {i}\n"))
            .collect();
        let mut modal = ToolsModal {
            rows: vec![ToolRow {
                name: "kg_upsert".into(),
                read_only: false,
                outbox: false,
                caps: Capabilities {
                    private_data: true,
                    ..Default::default()
                },
                description: long,
            }],
            selected: 0,
            detail: true,
            detail_scroll: 0,
            sandbox_line: "sandbox: none".into(),
        };

        let top = frame_text(&modal, 90, 22);
        assert!(top.contains("description line 1"), "{top}");
        assert!(
            !top.contains("reads data the user considers private"),
            "the capability block is genuinely below the fold: {top}"
        );
        // And the box says there is more, rather than looking complete.
        assert!(top.contains("↑↓ scrolls"), "{top}");

        modal.scroll_detail(99);
        let bottom = frame_text(&modal, 90, 22);
        assert!(
            bottom.contains("reads data the user considers private"),
            "scrolling reaches it: {bottom}"
        );
    }

    /// A short description needs no scrolling and must not advertise any —
    /// a hint that is always on screen is a hint nobody reads.
    #[test]
    fn a_description_that_fits_says_nothing_about_scrolling() {
        let modal = ToolsModal {
            rows: vec![ToolRow {
                name: "fs_read".into(),
                read_only: true,
                outbox: false,
                caps: Capabilities::default(),
                description: "Read a file.".into(),
            }],
            selected: 0,
            detail: true,
            detail_scroll: 0,
            sandbox_line: "sandbox: none".into(),
        };
        let text = frame_text(&modal, 90, 30);
        assert!(!text.contains("scrolls"), "{text}");
    }

    /// An offset carried onto another row is a position in a different
    /// document — the rule every sibling modal follows.
    #[test]
    fn moving_the_selection_resets_the_detail_scroll() {
        let mut modal = ToolsModal {
            rows: vec![
                row("a", Capabilities::default()),
                row("b", Capabilities::default()),
            ],
            selected: 0,
            detail: true,
            detail_scroll: 0,
            sandbox_line: String::new(),
        };
        modal.scroll_detail(12);
        assert_eq!(modal.detail_scroll, 12);
        modal.move_by(1);
        assert_eq!(modal.detail_scroll, 0);
        // And it never wraps below zero into a huge offset.
        modal.scroll_detail(-5);
        assert_eq!(modal.detail_scroll, 0);
    }

    fn row(name: &str, caps: Capabilities) -> ToolRow {
        ToolRow {
            name: name.into(),
            read_only: true,
            outbox: false,
            caps,
            description: "does things".into(),
        }
    }

    #[test]
    fn badges_name_the_declared_risks_and_nothing_else() {
        let quiet = row("fs_list", Capabilities::default());
        assert_eq!(quiet.badges(), "ro");

        let loud = ToolRow {
            read_only: false,
            outbox: true,
            ..row(
                "email_send",
                Capabilities {
                    external_send: true,
                    ..Capabilities::default()
                },
            )
        };
        assert_eq!(loud.badges(), "writes · outbox · sends");
    }

    #[test]
    fn the_selection_wraps_and_an_empty_list_does_not_panic() {
        let mut modal = ToolsModal {
            rows: vec![
                row("a", Capabilities::default()),
                row("b", Capabilities::default()),
            ],
            selected: 0,
            detail: false,
            detail_scroll: 0,
            sandbox_line: String::new(),
        };
        modal.move_by(-1);
        assert_eq!(modal.selected, 1, "did not wrap backwards");
        modal.move_by(1);
        assert_eq!(modal.selected, 0, "did not wrap forwards");

        let mut empty = ToolsModal {
            rows: Vec::new(),
            selected: 0,
            detail: false,
            detail_scroll: 0,
            sandbox_line: String::new(),
        };
        empty.move_by(1);
        assert_eq!(empty.selected, 0);
    }

    #[test]
    fn a_long_list_scrolls_to_keep_the_selection_visible() {
        let rows: Vec<ToolRow> = (0..30)
            .map(|i| row(&format!("t{i}"), Capabilities::default()))
            .collect();
        let modal = ToolsModal {
            rows,
            selected: 0,
            detail: false,
            detail_scroll: 0,
            sandbox_line: String::new(),
        };
        assert_eq!(
            modal.list_scroll(10),
            0,
            "selection at the top needs no scroll"
        );

        let modal = ToolsModal {
            selected: 25,
            ..modal
        };
        assert_eq!(
            modal.list_scroll(10),
            16,
            "selection stays on the last visible row"
        );
    }
    /// Fails on the old inline `clamp(1, height.saturating_sub(4))`, which
    /// panicked with `min > max` the moment the terminal was four rows or
    /// fewer — the /doctor bug (F9), which every modal had a copy of because
    /// each new one is written by opening whichever sibling is nearest.
    #[test]
    fn a_tiny_terminal_shrinks_the_list_rather_than_panicking() {
        let modal = ToolsModal {
            rows: vec![row("shell", Capabilities::default())],
            selected: 0,
            detail: false,
            detail_scroll: 0,
            sandbox_line: String::new(),
        };
        for height in 0..=6u16 {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, height.max(1)))
                    .unwrap();
            // The draw itself is the assertion: the old code panicked here.
            terminal.draw(|f| modal.draw(f)).unwrap();
        }
    }
}
