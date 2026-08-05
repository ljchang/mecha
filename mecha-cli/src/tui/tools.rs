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

        let height = (body.len() as u16).clamp(1, frame.area().height.saturating_sub(4)) + 2;
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
        let Some(row) = self.rows.get(self.selected) else { return };

        let mut body: Vec<Line> = vec![Line::styled(
            format!("[{}]", row.badges()),
            Style::new().fg(Color::DarkGray),
        )];
        body.push(Line::raw(""));
        for line in row.description.lines() {
            body.push(Line::styled(line.to_string(), Style::new().fg(Color::White)));
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

        let area = super::centered(frame.area(), 80, (body.len() as u16 + 4).min(frame.area().height));
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(body).wrap(Wrap { trim: false }).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(Color::Cyan))
                    .title(format!(" {} · esc to go back ", row.name)),
            ),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                Capabilities { external_send: true, ..Capabilities::default() },
            )
        };
        assert_eq!(loud.badges(), "writes · outbox · sends");
    }

    #[test]
    fn the_selection_wraps_and_an_empty_list_does_not_panic() {
        let mut modal = ToolsModal {
            rows: vec![row("a", Capabilities::default()), row("b", Capabilities::default())],
            selected: 0,
            detail: false,
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
            sandbox_line: String::new(),
        };
        empty.move_by(1);
        assert_eq!(empty.selected, 0);
    }

    #[test]
    fn a_long_list_scrolls_to_keep_the_selection_visible() {
        let rows: Vec<ToolRow> = (0..30).map(|i| row(&format!("t{i}"), Capabilities::default())).collect();
        let modal = ToolsModal { rows, selected: 0, detail: false, sandbox_line: String::new() };
        assert_eq!(modal.list_scroll(10), 0, "selection at the top needs no scroll");

        let modal = ToolsModal { selected: 25, ..modal };
        assert_eq!(modal.list_scroll(10), 16, "selection stays on the last visible row");
    }
}
