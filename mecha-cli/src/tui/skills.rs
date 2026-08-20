//! The /skills modal: what the agent knows how to do, as distinct from what
//! it can call.
//!
//! `/tools` answers half of "what can this agent actually do"; this is the
//! other half, in the same shape and for the same reason — a glance down a
//! list, then the procedure itself on the one you mean. `mecha skills` is the
//! command-line twin, and the two answer from different places on purpose:
//!
//! - **What the run carries comes from the running agent**, never from
//!   re-deriving the selection off config. `--skill` narrows a run without
//!   touching config or the store, and `mecha skills` shipped with exactly
//!   that bug — every config-selected skill marked as carried while the run
//!   carried one.
//! - **What exists on disk comes from the store**, because the agent only
//!   holds what survived selection. Without that read a withheld skill is
//!   indistinguishable from a skill the model chose not to use, which is the
//!   question this modal is most often opened to answer.
//! - **A skill that failed to parse is a row here**, not just a line on
//!   stderr. `setup` prints its warning before the TUI takes the screen, so
//!   the alternate screen covers it for the whole session and it comes back
//!   only on exit — this is the only surface where a TUI user can see a
//!   broken `SKILL.md` while there is still something to do about it.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use std::path::PathBuf;

pub struct SkillRow {
    pub name: String,
    pub description: String,
    /// Keyword hints from the frontmatter. Advisory — the model decides.
    pub triggers: Vec<String>,
    /// The `tools:` allowlist, when the skill declares one. Narrowing only:
    /// loading it restricts the surface for the rest of the conversation.
    pub narrows: Option<Vec<String>>,
    pub body: String,
    pub dir: PathBuf,
    /// In this run's level-1 block — config and `--skill` let it through.
    pub carried: bool,
    /// Loaded in this conversation, so its body is in context and any
    /// narrowing it declares is in force.
    pub loaded: bool,
    /// Why this `SKILL.md` did not load. `Some` makes the row a failure
    /// rather than a skill; the other fields are then empty.
    pub error: Option<String>,
}

impl SkillRow {
    /// The compact state summary beside the name. One badge, because unlike a
    /// tool's capabilities these are stages of one lifecycle rather than
    /// independent facts: on disk → carried → loaded.
    fn badge(&self) -> &'static str {
        if self.error.is_some() {
            "failed"
        } else if self.loaded {
            "loaded"
        } else if self.carried {
            "carried"
        } else {
            "withheld"
        }
    }

    fn colour(&self) -> Color {
        match self.badge() {
            "failed" => Color::Red,
            "loaded" => Color::Green,
            "withheld" => Color::DarkGray,
            _ => Color::White,
        }
    }
}

pub struct SkillsModal {
    pub rows: Vec<SkillRow>,
    pub selected: usize,
    /// Showing the procedure itself rather than the list.
    pub detail: bool,
    /// How far the detail view is scrolled.
    ///
    /// A `SKILL.md` is the one thing in this TUI with no length bound — it is
    /// a document the user wrote — so the detail view is the one that cannot
    /// get away without scrolling. `Wrap` also means the rendered height is
    /// not `body.len()`, so even a body that looks shorter than the screen can
    /// lose its tail. Reset on every move, like the outbox modal's: a scroll
    /// offset carried onto another row is pointing into a different document.
    pub detail_scroll: u16,
    /// The store these came from, shown in the title — "which directory am I
    /// even looking at" is half of every skill question, and `[skills] dir`
    /// can move it.
    pub dir: PathBuf,
}

impl SkillsModal {
    pub fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let len = self.rows.len() as isize;
        // Wraps, like the tools modal: a list is faster to cycle than to bound.
        self.selected = (((self.selected as isize + delta) % len + len) % len) as usize;
        self.detail_scroll = 0;
    }

    pub fn scroll_detail(&mut self, delta: i16) {
        self.detail_scroll = self.detail_scroll.saturating_add_signed(delta);
    }

    /// Enter opens the procedure — but only where there is one.
    ///
    /// `draw_detail` renders nothing at all when the selection names no row,
    /// not even a `Clear`, so toggling into the detail of an empty store makes
    /// the modal *invisible while it still owns the keyboard*: the box
    /// vanishes and every keystroke is swallowed until esc is pressed twice.
    /// A modal that is up must always be on screen.
    pub fn toggle_detail(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        self.detail = !self.detail;
        self.detail_scroll = 0;
    }

    pub fn draw(&self, frame: &mut Frame) {
        if self.detail {
            self.draw_detail(frame);
        } else {
            self.draw_list(frame);
        }
    }

    /// The title line, which carries the count that matters.
    ///
    /// Carried-of-total rather than a bare total: a store of nine skills where
    /// the run carries two is the single most confusing state this modal
    /// exists to explain, and a list that just looks long does not explain it.
    fn title(&self) -> String {
        if self.rows.is_empty() {
            return format!(" no skills in {} ", self.dir.display());
        }
        // A row that did not parse is not a skill the run could have carried,
        // so it counts in neither half of the ratio — it gets its own clause.
        // `badge()` makes a failure dominate the other flags for the same
        // reason, and the two must agree or the list and the title describe
        // different stores.
        let (loadable, carried) = self
            .rows
            .iter()
            .filter(|r| r.error.is_none())
            .fold((0, 0), |(n, c), r| (n + 1, c + usize::from(r.carried)));
        let failed = self.rows.iter().filter(|r| r.error.is_some()).count();
        let failed = if failed == 0 {
            String::new()
        } else {
            format!(" · {failed} failed to load")
        };
        format!(
            " {carried} of {loadable} skills carried{failed} · enter for the procedure, esc to close ",
        )
    }

    fn draw_list(&self, frame: &mut Frame) {
        // An empty store is a real answer and deserves the instructions,
        // rather than an empty box that reads as something being broken.
        if self.rows.is_empty() {
            let body = vec![
                Line::styled(
                    "a skill is a directory holding a SKILL.md — YAML frontmatter with",
                    Style::new().fg(Color::White),
                ),
                Line::styled(
                    "`name` and `description`, then the procedure as markdown",
                    Style::new().fg(Color::White),
                ),
                Line::raw(""),
                Line::styled(
                    "skills are read once at startup, so a new one needs a restart",
                    Style::new().fg(Color::DarkGray),
                ),
            ];
            let area = super::centered(frame.area(), 80, body.len() as u16 + 2);
            frame.render_widget(Clear, area);
            frame.render_widget(
                Paragraph::new(body).wrap(Wrap { trim: false }).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::new().fg(Color::Cyan))
                        .title(self.title()),
                ),
                area,
            );
            return;
        }

        let body: Vec<Line> = self
            .rows
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let summary = row
                    .error
                    .as_deref()
                    .unwrap_or(&row.description)
                    .lines()
                    .next()
                    .unwrap_or("");
                let text = format!(
                    "{} {:<22} [{:<8}] {}",
                    if i == self.selected { "›" } else { " " },
                    row.name,
                    row.badge(),
                    summary,
                );
                if i == self.selected {
                    Line::styled(text, Style::new().fg(Color::Black).bg(Color::Cyan))
                } else {
                    Line::styled(text, Style::new().fg(row.colour()))
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
                        .title(self.title()),
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

        let mut body: Vec<Line> = Vec::new();

        if let Some(why) = &row.error {
            body.push(Line::styled(
                format!("this SKILL.md did not load — {why}"),
                Style::new().fg(Color::Red),
            ));
            body.push(Line::raw(""));
            body.push(Line::styled(
                row.dir.display().to_string(),
                Style::new().fg(Color::DarkGray),
            ));
            body.push(Line::raw(""));
            body.push(Line::styled(
                "unknown frontmatter keys are ignored so a skill written for another \
                 harness still loads; a known key with the wrong type is refused, because \
                 that is an authoring mistake rather than a portability one",
                Style::new().fg(Color::DarkGray),
            ));
            self.render_detail(frame, &row.name, body);
            return;
        }

        body.push(Line::styled(
            format!("[{}]", row.badge()),
            Style::new().fg(Color::DarkGray),
        ));
        body.push(Line::raw(""));
        body.push(Line::styled(
            row.description.clone(),
            Style::new().fg(Color::White),
        ));
        body.push(Line::raw(""));

        if !row.triggers.is_empty() {
            body.push(Line::styled(
                format!("• keywords: {}", row.triggers.join(", ")),
                Style::new().fg(Color::DarkGray),
            ));
        }
        if let Some(tools) = &row.narrows {
            // Worth a colour: this is the one thing loading a skill does to
            // the rest of the conversation, and there is no unload.
            body.push(Line::styled(
                format!("• narrows the tool surface to: {}", tools.join(", ")),
                Style::new().fg(Color::Yellow),
            ));
        }
        if !row.carried {
            body.push(Line::styled(
                "• withheld from this run by [skills] or --skill — the model cannot load it",
                Style::new().fg(Color::Yellow),
            ));
        }
        if row.loaded {
            body.push(Line::styled(
                "• loaded in this conversation: the procedure below is in context, and \
                 any narrowing above is in force until /clear",
                Style::new().fg(Color::Green),
            ));
        }
        body.push(Line::raw(""));

        // The body, exactly as the model receives it. `mecha skills --show`
        // prints the same bytes for the same reason: a procedure you cannot
        // read is one you cannot debug.
        for line in row.body.lines() {
            body.push(Line::styled(
                format!("│ {line}"),
                Style::new().fg(Color::White),
            ));
        }

        self.render_detail(frame, &row.name, body);
    }

    fn render_detail(&self, frame: &mut Frame, name: &str, body: Vec<Line>) {
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
                        .title(format!(" {name} · ↑↓ scrolls · esc to go back ")),
                ),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;

    fn row(name: &str) -> SkillRow {
        SkillRow {
            name: name.into(),
            description: "does a thing".into(),
            triggers: Vec::new(),
            narrows: None,
            body: "step one".into(),
            dir: PathBuf::from("/skills").join(name),
            carried: true,
            loaded: false,
            error: None,
        }
    }

    #[test]
    fn the_badge_names_the_furthest_stage_reached() {
        assert_eq!(row("a").badge(), "carried");

        let loaded = SkillRow {
            loaded: true,
            ..row("a")
        };
        assert_eq!(loaded.badge(), "loaded");

        // Withheld beats carried-is-false being merely absent: the whole
        // point is that it is on disk and still did not make the run.
        let withheld = SkillRow {
            carried: false,
            ..row("a")
        };
        assert_eq!(withheld.badge(), "withheld");

        // A failure is a failure however the other flags read — a row that
        // did not parse cannot honestly claim to be carried.
        let failed = SkillRow {
            error: Some("missing `description`".into()),
            loaded: true,
            ..row("a")
        };
        assert_eq!(failed.badge(), "failed");
    }

    #[test]
    fn the_title_counts_carried_out_of_loadable_and_names_failures_apart() {
        let modal = SkillsModal {
            rows: vec![
                row("a"),
                SkillRow {
                    carried: false,
                    ..row("b")
                },
                SkillRow {
                    error: Some("bad yaml".into()),
                    ..row("c")
                },
            ],
            selected: 0,
            detail: false,
            detail_scroll: 0,
            dir: PathBuf::from("/skills"),
        };
        let title = modal.title();
        assert!(
            title.contains("1 of 2 skills carried"),
            "a failed load is not a skill the run could have carried: {title}"
        );
        assert!(title.contains("1 failed to load"), "{title}");
    }

    #[test]
    fn an_empty_store_says_so_in_the_title() {
        let modal = SkillsModal {
            rows: Vec::new(),
            selected: 0,
            detail: false,
            detail_scroll: 0,
            dir: PathBuf::from("/nowhere"),
        };
        assert!(modal.title().contains("no skills in /nowhere"));
    }

    /// Fails on the old inline `clamp(1, height.saturating_sub(4))`, which
    /// panicked with `min > max` the moment the terminal was four rows or
    /// fewer — the /doctor bug (F9) rewritten in a new module.
    #[test]
    fn a_tiny_terminal_shrinks_the_list_rather_than_panicking() {
        let modal = SkillsModal {
            rows: vec![row("a"), row("b"), row("c")],
            selected: 0,
            detail: false,
            detail_scroll: 0,
            dir: PathBuf::from("/skills"),
        };
        for height in 0..=6u16 {
            let mut terminal =
                Terminal::new(ratatui::backend::TestBackend::new(100, height.max(1))).unwrap();
            // The draw itself is the assertion: the old code panicked here.
            terminal.draw(|f| modal.draw(f)).unwrap();
        }
    }

    /// Enter on an empty store used to flip into a detail view that renders
    /// nothing at all — not even a `Clear` — leaving an invisible modal that
    /// still swallowed every keypress.
    #[test]
    fn an_empty_store_cannot_be_toggled_into_an_invisible_detail() {
        let mut modal = SkillsModal {
            rows: Vec::new(),
            selected: 0,
            detail: false,
            detail_scroll: 0,
            dir: PathBuf::from("/skills"),
        };
        modal.toggle_detail();
        assert!(!modal.detail, "a modal that is up must stay on screen");
    }

    /// A scroll offset is a position in one document; carrying it onto the
    /// next row points into a different one.
    #[test]
    fn moving_resets_the_detail_scroll() {
        let mut modal = SkillsModal {
            rows: vec![row("a"), row("b")],
            selected: 0,
            detail: false,
            detail_scroll: 0,
            dir: PathBuf::from("/skills"),
        };
        modal.scroll_detail(12);
        assert_eq!(modal.detail_scroll, 12);
        modal.move_by(1);
        assert_eq!(modal.detail_scroll, 0);

        // And it never wraps below zero into a huge offset.
        modal.scroll_detail(-5);
        assert_eq!(modal.detail_scroll, 0);
    }

    #[test]
    fn the_selection_wraps_and_an_empty_list_does_not_panic() {
        let mut modal = SkillsModal {
            rows: vec![row("a"), row("b")],
            selected: 0,
            detail: false,
            detail_scroll: 0,
            dir: PathBuf::from("/skills"),
        };
        modal.move_by(-1);
        assert_eq!(modal.selected, 1, "did not wrap backwards");
        modal.move_by(1);
        assert_eq!(modal.selected, 0, "did not wrap forwards");

        let mut empty = SkillsModal {
            rows: Vec::new(),
            selected: 0,
            detail: false,
            detail_scroll: 0,
            dir: PathBuf::from("/skills"),
        };
        empty.move_by(1);
        assert_eq!(empty.selected, 0);
    }

    #[test]
    fn a_long_list_scrolls_to_keep_the_selection_visible() {
        let rows: Vec<SkillRow> = (0..30).map(|i| row(&format!("s{i}"))).collect();
        let modal = SkillsModal {
            rows,
            selected: 25,
            detail: false,
            detail_scroll: 0,
            dir: PathBuf::from("/skills"),
        };
        assert_eq!(
            modal.list_scroll(10),
            16,
            "selection stays on the last visible row"
        );
    }
}
