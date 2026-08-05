//! The /triggers modal: what is scheduled, and what to do about it.
//!
//! Two depths, like /tools: the list answers "what runs, and when next", and
//! the detail answers "what does this one actually ask, how did it go, and
//! what did it say last time". The last part is why the detail reads the
//! *session transcript* rather than any store of its own — the transcript is
//! the record of what a run produced, and a second copy could disagree with
//! it.
//!
//! Every action here shells out to `mecha trigger ...` as a child process
//! rather than reaching into the store directly. That is deliberate and worth
//! keeping: firing a trigger builds a whole separate agent (its own provider,
//! tool surface, workspace and budgets) and can take twenty minutes, and doing
//! that on the TUI's event loop would freeze the interface for the duration.
//! Going through the CLI means one implementation of firing, no way for the
//! TUI to do something the command line cannot, and a run that outlives the
//! session that started it.

use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

pub struct TriggerRow {
    pub name: String,
    pub schedule: String,
    pub enabled: bool,
    pub description: Option<String>,
    /// "in 18h 6m (Thu 6 Aug 07:00 EDT)", or why not.
    pub when: String,
    /// One line about the most recent run, empty if it has never run.
    pub last: String,
    /// A run is in flight right now.
    pub running: bool,
    /// The prompt, wrapped into the detail view.
    pub prompt: String,
    /// Settings worth seeing before you trust it unattended: permission mode,
    /// tool surface, budgets.
    pub settings: Vec<String>,
    /// Recent ledger rows, newest last.
    pub runs: Vec<String>,
    /// What the last run answered, read back from its transcript.
    pub last_answer: Option<String>,
}

impl TriggerRow {
    fn badge(&self) -> &'static str {
        if self.running {
            "running"
        } else if self.enabled {
            "on"
        } else {
            "off"
        }
    }

    fn badge_style(&self) -> Style {
        if self.running {
            Style::new().fg(Color::Cyan)
        } else if self.enabled {
            Style::new().fg(Color::Green)
        } else {
            Style::new().fg(Color::DarkGray)
        }
    }
}

/// A destructive action waiting on a yes.
pub struct Confirm {
    pub name: String,
    pub prompt: String,
}

pub struct TriggersModal {
    pub rows: Vec<TriggerRow>,
    pub selected: usize,
    pub detail: bool,
    /// Scroll offset within the detail view — a briefing is longer than a
    /// modal.
    pub detail_scroll: u16,
    /// Deleting needs a yes. Nothing else here does: enable/disable and run
    /// are all reversible, and a confirmation on a reversible action teaches
    /// people to hit y without reading.
    pub confirm: Option<Confirm>,
    /// The result of the last action, shown in the title bar.
    pub status: Option<String>,
}

impl TriggersModal {
    pub fn new(rows: Vec<TriggerRow>) -> Self {
        TriggersModal {
            rows,
            selected: 0,
            detail: false,
            detail_scroll: 0,
            confirm: None,
            status: None,
        }
    }

    pub fn selected_name(&self) -> Option<&str> {
        self.rows.get(self.selected).map(|r| r.name.as_str())
    }

    pub fn selected_row(&self) -> Option<&TriggerRow> {
        self.rows.get(self.selected)
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let len = self.rows.len() as isize;
        self.selected = (((self.selected as isize + delta) % len + len) % len) as usize;
        self.detail_scroll = 0;
    }

    pub fn scroll_detail(&mut self, delta: i16) {
        self.detail_scroll = self.detail_scroll.saturating_add_signed(delta);
    }

    /// Keep the selection visible when the list outgrows the modal.
    fn list_scroll(&self, visible: u16) -> u16 {
        let visible = visible.max(1) as usize;
        (self.selected + 1).saturating_sub(visible) as u16
    }

    pub fn draw(&self, frame: &mut Frame) {
        if self.confirm.is_some() {
            self.draw_confirm(frame);
        } else if self.detail {
            self.draw_detail(frame);
        } else {
            self.draw_list(frame);
        }
    }

    fn draw_list(&self, frame: &mut Frame) {
        let body: Vec<Line> = if self.rows.is_empty() {
            vec![Line::styled(
                "  no triggers — mecha trigger add <name> --schedule '0 7 * * *' --prompt '…'",
                Style::new().fg(Color::DarkGray),
            )]
        } else {
            self.rows
                .iter()
                .enumerate()
                .map(|(i, row)| {
                    let selected = i == self.selected;
                    let marker = if selected { "›" } else { " " };
                    let text = format!(
                        "{marker} {:<18} {:<14} {:<9} {:<30} {}",
                        row.name,
                        row.schedule,
                        row.badge(),
                        row.when,
                        row.last
                    );
                    if selected {
                        Line::styled(text, Style::new().fg(Color::Black).bg(Color::Cyan))
                    } else {
                        Line::styled(text, Style::new().fg(Color::White))
                    }
                })
                .collect()
        };

        let height = (body.len() as u16).clamp(1, frame.area().height.saturating_sub(4)) + 2;
        let area = super::centered(frame.area(), 110, height);
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

    /// The title carries the keymap, because a modal whose actions are
    /// invisible is a modal with one action.
    fn title(&self) -> String {
        match &self.status {
            Some(s) => format!(" triggers · {s} "),
            None => {
                let cancel = if self.rows.iter().any(|r| r.running) { " c cancel ·" } else { "" };
                format!(
                    " {} trigger(s) · enter detail · e edit · space on/off · r run now ·{cancel} x delete · esc ",
                    self.rows.len()
                )
            }
        }
    }

    fn draw_detail(&self, frame: &mut Frame) {
        let Some(row) = self.rows.get(self.selected) else { return };
        let mut body: Vec<Line> = Vec::new();

        body.push(Line::styled(
            format!("{}  [{}]", row.schedule, row.badge()),
            row.badge_style(),
        ));
        if let Some(d) = &row.description {
            body.push(Line::styled(d.clone(), Style::new().fg(Color::White)));
        }
        body.push(Line::styled(row.when.clone(), Style::new().fg(Color::DarkGray)));
        body.push(Line::raw(""));

        for line in &row.settings {
            body.push(Line::styled(line.clone(), Style::new().fg(Color::DarkGray)));
        }
        body.push(Line::raw(""));

        body.push(Line::styled("prompt", Style::new().fg(Color::Yellow)));
        for line in row.prompt.lines() {
            body.push(Line::styled(line.to_string(), Style::new().fg(Color::White)));
        }

        if !row.runs.is_empty() {
            body.push(Line::raw(""));
            body.push(Line::styled("recent runs", Style::new().fg(Color::Yellow)));
            for line in &row.runs {
                body.push(Line::styled(line.clone(), Style::new().fg(Color::White)));
            }
        }

        if let Some(answer) = &row.last_answer {
            body.push(Line::raw(""));
            body.push(Line::styled("last answer", Style::new().fg(Color::Yellow)));
            for line in answer.lines() {
                body.push(Line::styled(line.to_string(), Style::new().fg(Color::White)));
            }
        }

        let area = super::centered(frame.area(), 100, frame.area().height.saturating_sub(4));
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(body)
                .wrap(Wrap { trim: false })
                .scroll((self.detail_scroll, 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::new().fg(Color::Cyan))
                        .title(format!(" {} · ↑↓ scroll · e edit · esc back ", row.name)),
                ),
            area,
        );
    }

    fn draw_confirm(&self, frame: &mut Frame) {
        let Some(confirm) = &self.confirm else { return };
        let body = vec![
            Line::styled(confirm.prompt.clone(), Style::new().fg(Color::White)),
            Line::raw(""),
            Line::styled("y to confirm · anything else cancels", Style::new().fg(Color::DarkGray)),
        ];
        let area = super::centered(frame.area(), 70, 5);
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(body).wrap(Wrap { trim: false }).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(Color::Red))
                    .title(" confirm "),
            ),
            area,
        );
    }
}

/// Build the rows from the store. Pure enough to be worth keeping out of the
/// event loop: everything here is a file read.
pub fn load(limit_runs: usize) -> anyhow::Result<Vec<TriggerRow>> {
    use mecha_core::trigger::TriggerStore;

    let store = TriggerStore::open_default()?;
    let (triggers, _problems) = store.list()?;
    let last_slots = store.last_slots()?;
    let all_runs = store.runs()?;
    let tz_default = mecha_core::config::Config::load_global().ok().and_then(|c| c.agent.timezone());
    let now = Utc::now();

    let mut rows = Vec::new();
    for t in triggers {
        let tz = t.tz(tz_default);
        let when = if !t.enabled {
            "disabled".to_string()
        } else {
            match t.due(last_slots.get(&t.name).copied(), now, tz_default) {
                mecha_core::trigger::Due::Now { .. }
                | mecha_core::trigger::Due::Stale { .. } => "due now".to_string(),
                mecha_core::trigger::Due::Not { next: Some(next) } => {
                    format!("in {} ({})", gap(next - now), local(next, tz))
                }
                _ => "never".to_string(),
            }
        };

        let mine: Vec<_> = all_runs.iter().filter(|r| r.trigger == t.name).collect();
        let last = mine
            .last()
            .map(|r| {
                format!(
                    "last {} {} ago",
                    r.status.as_str(),
                    gap(now - r.started_at)
                )
            })
            .unwrap_or_default();

        let mut settings = vec![
            format!("permission {:?} · timeout {} · catch up {}",
                t.permission_mode,
                mecha_core::trigger::render_duration(t.timeout_duration()),
                t.catch_up),
        ];
        if let Some(p) = &t.provider {
            settings.push(format!("provider {p}"));
        }
        if let Some(w) = &t.workspace {
            settings.push(format!("workspace {}", w.display()));
        }
        if !t.tools.is_empty() {
            settings.push(format!("tools {}", t.tools.join(", ")));
        }
        if let Some(n) = &t.notify {
            settings.push(format!("notify {n}"));
        }

        let runs: Vec<String> = mine
            .iter()
            .rev()
            .take(limit_runs)
            .map(|r| {
                let mut line = format!(
                    "{}  {}",
                    r.started_at.with_timezone(&tz).format("%d %b %H:%M"),
                    r.status.as_str()
                );
                if r.manual {
                    line.push_str(" manual");
                }
                if r.staged > 0 {
                    line.push_str(&format!(" · {} staged", r.staged));
                }
                if let Some(e) = &r.error {
                    line.push_str(&format!(" · {e}"));
                }
                line
            })
            .collect();

        let last_answer = mine
            .iter()
            .rev()
            .find(|r| r.session_id.is_some())
            .and_then(|r| read_answer(r.session_id.as_deref().unwrap_or_default()));

        rows.push(TriggerRow {
            running: store.running(&t.name).is_some(),
            name: t.name.clone(),
            schedule: t.schedule.source().to_string(),
            enabled: t.enabled,
            description: t.description.clone(),
            when,
            last,
            prompt: t.prompt.clone(),
            settings,
            runs,
            last_answer,
        });
    }
    Ok(rows)
}

/// The final assistant turn of a recorded run. Best-effort: a session that has
/// been cleaned up should grey out the answer, not fail the modal.
fn read_answer(session_id: &str) -> Option<String> {
    let dir = mecha_core::session::Session::default_dir().ok()?;
    let path = mecha_core::session::Session::find(&dir, session_id).ok()?;
    let (_, convo) = mecha_core::session::Session::load(&path).ok()?;
    convo
        .messages
        .iter()
        .rev()
        .find(|m| m.role == mecha_core::Role::Assistant)
        .map(|m| m.text())
        .filter(|t| !t.trim().is_empty())
}

fn local(at: DateTime<Utc>, tz: chrono_tz::Tz) -> String {
    at.with_timezone(&tz).format("%a %-d %b %H:%M %Z").to_string()
}

fn gap(d: chrono::Duration) -> String {
    let secs = d.num_seconds().abs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d {}h", secs / 86_400, (secs % 86_400) / 3600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str) -> TriggerRow {
        TriggerRow {
            name: name.into(),
            schedule: "0 7 * * *".into(),
            enabled: true,
            description: None,
            when: "in 3h".into(),
            last: String::new(),
            running: false,
            prompt: "brief me".into(),
            settings: Vec::new(),
            runs: Vec::new(),
            last_answer: None,
        }
    }

    #[test]
    fn the_badge_says_which_of_the_three_states_it_is_in() {
        assert_eq!(row("a").badge(), "on");
        assert_eq!(TriggerRow { enabled: false, ..row("a") }.badge(), "off");
        // Running wins over enabled: what it is doing beats what it will do.
        assert_eq!(TriggerRow { running: true, ..row("a") }.badge(), "running");
        assert_eq!(
            TriggerRow { running: true, enabled: false, ..row("a") }.badge(),
            "running",
            "a run started before it was disabled is still a run"
        );
    }

    #[test]
    fn the_selection_wraps_and_an_empty_list_does_not_panic() {
        let mut modal = TriggersModal::new(vec![row("a"), row("b")]);
        modal.move_by(-1);
        assert_eq!(modal.selected, 1);
        modal.move_by(1);
        assert_eq!(modal.selected, 0);

        let mut empty = TriggersModal::new(Vec::new());
        empty.move_by(1);
        assert_eq!(empty.selected, 0);
        assert_eq!(empty.selected_name(), None);
    }

    #[test]
    fn moving_resets_the_detail_scroll() {
        // Otherwise the next trigger opens scrolled into the middle of a
        // briefing that is not there.
        let mut modal = TriggersModal::new(vec![row("a"), row("b")]);
        modal.scroll_detail(20);
        assert_eq!(modal.detail_scroll, 20);
        modal.move_by(1);
        assert_eq!(modal.detail_scroll, 0);
    }

    #[test]
    fn scrolling_up_past_the_top_stops_rather_than_wrapping_around() {
        let mut modal = TriggersModal::new(vec![row("a")]);
        modal.scroll_detail(-5);
        assert_eq!(modal.detail_scroll, 0, "a u16 that wrapped would scroll to the end");
    }

    /// The keymap has to be on screen: a modal whose actions are invisible is
    /// a modal with one action.
    #[test]
    fn the_title_shows_the_keys_and_offers_cancel_only_when_something_runs() {
        let idle = TriggersModal::new(vec![row("a")]);
        let title = idle.title();
        for key in ["enter", "e edit", "space on/off", "r run now", "x delete", "esc"] {
            assert!(title.contains(key), "{key} missing from {title}");
        }
        assert!(!title.contains("c cancel"), "nothing is running: {title}");

        let busy = TriggersModal::new(vec![TriggerRow { running: true, ..row("a") }]);
        assert!(busy.title().contains("c cancel"), "{}", busy.title());

        // A status message replaces the keymap — it is the answer to what the
        // user just pressed, which is what they are looking for.
        let done = TriggersModal { status: Some("started `a`".into()), ..idle };
        assert!(done.title().contains("started `a`"));
    }

    #[test]
    fn a_long_list_scrolls_to_keep_the_selection_visible() {
        let rows: Vec<TriggerRow> = (0..30).map(|i| row(&format!("t{i}"))).collect();
        let modal = TriggersModal::new(rows);
        assert_eq!(modal.list_scroll(10), 0);
        let modal = TriggersModal { selected: 25, ..modal };
        assert_eq!(modal.list_scroll(10), 16);
    }
}
