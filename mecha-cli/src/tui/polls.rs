//! The /polls modal: open polls, their tallies, and the lecture controls,
//! without leaving the session.
//!
//! The `/outbox`/`/frontdoor` pattern with one honest difference: the store
//! of record is on the **gate**, not on this machine. The list draws from
//! the local creation records (`~/.mecha/factory/polls/*.json` — who was
//! invited, which the box never learns), and everything live arrives by
//! driving `factory-publish polls …` as a child process — the same output
//! the CLI prints, shown verbatim, never a second tally implementation. So
//! the modal states its staleness ("as of 14:03:22") and an unreachable
//! gate is a labelled condition on the row, not a blank panel.
//!
//! Text answers surface here on purpose: the presenter's own screen is
//! where prose belongs (the projector gets the word cloud), and a person
//! reading it in a terminal is the safe context — drawn and dropped,
//! nothing rendered in a modal reaches a model.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

pub struct PollRow {
    pub instrument: String,
    pub poll_id: String,
    pub title: String,
    /// "12 people" or "link" — who may answer, from the creation record.
    pub audience: String,
    pub deadline: String,
    pub created_at: String,
    pub screen_url: Option<String>,
    /// The gate's last answer, once fetched.
    pub live: Option<Live>,
}

pub struct Live {
    /// Local wall-clock of the fetch — the staleness the title admits to.
    pub as_of: String,
    pub ok: bool,
    /// "open · 2 of 5 answered", parsed from the CLI's first line; or the
    /// failure, named.
    pub summary: String,
    /// The CLI's own output (or error), line for line.
    pub lines: Vec<Line<'static>>,
}

/// The optional outcome typed before a close. Empty closes without one —
/// unlike the frontdoor's reason, a resolution is Loomio's outcome
/// statement, not an accountability requirement.
pub struct ResolutionInput {
    pub poll_id: String,
    pub buffer: String,
}

impl ResolutionInput {
    pub fn title(&self) -> String {
        format!(
            " close {} — outcome for the page (optional) · enter closes · esc back ",
            self.poll_id
        )
    }
}

pub struct PollsModal {
    pub rows: Vec<PollRow>,
    pub selected: usize,
    pub detail: bool,
    pub detail_scroll: u16,
    pub input: Option<ResolutionInput>,
    pub status: Option<String>,
}

impl PollsModal {
    pub fn new(rows: Vec<PollRow>) -> Self {
        PollsModal {
            rows,
            selected: 0,
            detail: false,
            detail_scroll: 0,
            input: None,
            status: None,
        }
    }

    pub fn selected_row(&self) -> Option<&PollRow> {
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

    fn list_scroll(&self, visible: u16) -> u16 {
        let visible = visible.max(1) as usize;
        (self.selected + 1).saturating_sub(visible) as u16
    }

    pub fn title(&self) -> String {
        match &self.status {
            Some(s) => format!(" polls · {s} "),
            None => format!(
                " {} poll(s) · enter tallies · r refresh · c close · e export · s screen url · esc ",
                self.rows.len()
            ),
        }
    }

    pub fn draw(&self, frame: &mut Frame) {
        if let Some(input) = &self.input {
            super::outbox::draw_reason_input(frame, &input.title(), &input.buffer);
        } else if self.detail {
            self.draw_detail(frame);
        } else {
            self.draw_list(frame);
        }
    }

    fn draw_list(&self, frame: &mut Frame) {
        let body: Vec<Line> = if self.rows.is_empty() {
            vec![Line::styled(
                "  no polls on record — `factory-publish polls create` makes one",
                Style::new().fg(Color::DarkGray),
            )]
        } else {
            self.rows
                .iter()
                .enumerate()
                .map(|(i, row)| {
                    let selected = i == self.selected;
                    let marker = if selected { "›" } else { " " };
                    let live = match &row.live {
                        Some(live) => format!("{}  (as of {})", live.summary, live.as_of),
                        None => "· enter fetches the tally".into(),
                    };
                    let text = format!(
                        "{marker} {:<18} {:<10} {:<22} {}",
                        row.poll_id, row.audience, row.title_short(), live,
                    );
                    let unreachable =
                        row.live.as_ref().is_some_and(|l| !l.ok);
                    if selected {
                        Line::styled(text, Style::new().fg(Color::Black).bg(Color::Cyan))
                    } else if unreachable {
                        Line::styled(text, Style::new().fg(Color::Red))
                    } else {
                        Line::styled(text, Style::new().fg(Color::White))
                    }
                })
                .collect()
        };

        let height = (body.len() as u16).clamp(1, frame.area().height.saturating_sub(4)) + 2;
        let area = super::centered(frame.area(), 120, height);
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

    fn draw_detail(&self, frame: &mut Frame) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        let area = super::centered(frame.area(), 110, frame.area().height.saturating_sub(4));
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(row.detail_lines())
                .wrap(Wrap { trim: false })
                .scroll((self.detail_scroll, 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::new().fg(Color::Cyan))
                        .title(format!(
                            " {} · ↑↓ scroll · r refresh · c close · e export · esc back ",
                            row.poll_id
                        )),
                ),
            area,
        );
    }
}

impl PollRow {
    fn title_short(&self) -> String {
        if self.title.chars().count() <= 22 {
            self.title.clone()
        } else {
            let cut: String = self.title.chars().take(21).collect();
            format!("{cut}…")
        }
    }

    /// The detail view: the record's facts, then the gate's answer verbatim
    /// under the staleness it was fetched at.
    pub fn detail_lines(&self) -> Vec<Line<'static>> {
        let white = Style::new().fg(Color::White);
        let grey = Style::new().fg(Color::DarkGray);
        let header = Style::new().fg(Color::Yellow);
        let warn = Style::new().fg(Color::Red);

        let mut body = vec![
            Line::styled(self.title.clone(), white),
            Line::styled(
                format!(
                    "{} · {} · created {} · closes {}",
                    self.instrument, self.audience, self.created_at, self.deadline
                ),
                grey,
            ),
        ];
        if let Some(screen) = &self.screen_url {
            body.push(Line::styled(format!("projector: {screen}"), grey));
        }
        body.push(Line::raw(""));
        match &self.live {
            Some(live) => {
                body.push(Line::styled(
                    format!(
                        "the gate's answer, as of {} — r refreshes",
                        live.as_of
                    ),
                    header,
                ));
                body.extend(live.lines.iter().cloned());
            }
            None => body.push(Line::styled(
                "not fetched yet — r asks the gate".to_string(),
                warn,
            )),
        }
        body
    }

    /// Install a fetch's outcome. The summary for the list is the CLI's
    /// own first line with the poll id dropped; a failure is the failure.
    pub fn install_fetch(&mut self, as_of: String, result: anyhow::Result<String>) {
        self.live = Some(match result {
            Ok(text) => {
                let summary = text
                    .lines()
                    .next()
                    .and_then(|first| first.split_once("): ").map(|(state, rest)| {
                        let state = state.rsplit_once('(').map(|(_, s)| s).unwrap_or("?");
                        format!("{state} · {rest}")
                    }))
                    .unwrap_or_else(|| "fetched".into());
                Live {
                    as_of,
                    ok: true,
                    summary,
                    lines: text
                        .lines()
                        .map(|l| Line::styled(l.to_string(), Style::new().fg(Color::White)))
                        .collect(),
                }
            }
            Err(e) => Live {
                as_of,
                ok: false,
                summary: format!("gate unreachable — {e:#}"),
                lines: vec![Line::styled(
                    format!("gate unreachable — {e:#}"),
                    Style::new().fg(Color::Red),
                )],
            },
        });
    }
}

/// The creation records, newest first. The directory not existing is not an
/// error — it is a machine that has never created a poll.
pub fn load() -> anyhow::Result<Vec<PollRow>> {
    let dir = mecha_core::work::mecha_home()?.join("factory").join("polls");
    let mut rows = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(rows),
        Err(e) => return Err(e).context(format!("reading {}", dir.display())),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        // `.links.csv` lives beside the records; anything unreadable is
        // skipped rather than sinking the list — one torn file must not
        // hide every poll.
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(record) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if let Some(row) = row(&record) {
            rows.push(row);
        }
    }
    rows.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(rows)
}

use anyhow::Context;

fn row(record: &serde_json::Value) -> Option<PollRow> {
    let poll_id = record["poll_id"].as_str()?.to_string();
    let audience = match record["audience"].as_str() {
        Some("link") => "link".to_string(),
        _ => match record["participants"].as_array() {
            Some(list) => format!("{} people", list.len()),
            None => "?".into(),
        },
    };
    Some(PollRow {
        instrument: record["instrument"].as_str().unwrap_or("book").to_string(),
        poll_id,
        title: record["title"].as_str().unwrap_or("(untitled)").to_string(),
        audience,
        deadline: record["deadline"].as_str().unwrap_or("—").to_string(),
        created_at: record["created_at"].as_str().unwrap_or("").to_string(),
        screen_url: record["screen_url"].as_str().map(str::to_string),
        live: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> serde_json::Value {
        serde_json::json!({
            "instrument": "book",
            "poll_id": "psyc60-mid",
            "title": "PSYC 60 — mid-semester feedback",
            "deadline": "2026-10-16T23:59:00-04:00",
            "created_at": "2026-08-09T15:00:00Z",
            "participants": [{"name": "Priya", "email": "p@x.edu", "url": "https://…"}],
        })
    }

    fn text(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_title_names_the_keys_or_answers_the_last_action() {
        let modal = PollsModal::new(vec![row(&record()).unwrap()]);
        let title = modal.title();
        for key in ["enter", "r refresh", "c close", "e export", "s screen", "esc"] {
            assert!(title.contains(key), "{key} missing from {title}");
        }
        let done = PollsModal {
            status: Some("closed psyc60-mid".into()),
            ..modal
        };
        assert!(done.title().contains("closed psyc60-mid"));
    }

    #[test]
    fn a_link_record_and_a_roster_record_name_their_audiences() {
        let roster = row(&record()).unwrap();
        assert_eq!(roster.audience, "1 people");
        let mut linked = record();
        linked["audience"] = "link".into();
        assert_eq!(row(&linked).unwrap().audience, "link");
    }

    /// The detail is the CLI's own output under an admitted staleness —
    /// never a second rendering of the tallies.
    #[test]
    fn the_detail_carries_the_gates_answer_verbatim_with_its_age() {
        let mut poll = row(&record()).unwrap();
        poll.install_fetch(
            "14:03:22".into(),
            Ok("poll `psyc60-mid` (open): 2 of 5 answered\n\nPick one.\n    2  World models"
                .into()),
        );
        let live = poll.live.as_ref().unwrap();
        assert_eq!(live.summary, "open · 2 of 5 answered");
        let body = text(&poll.detail_lines());
        assert!(body.contains("as of 14:03:22"), "{body}");
        assert!(body.contains("    2  World models"), "{body}");
    }

    #[test]
    fn an_unreachable_gate_is_a_labelled_condition_not_a_blank() {
        let mut poll = row(&record()).unwrap();
        poll.install_fetch("14:03:22".into(), Err(anyhow::anyhow!("no route to gate")));
        let live = poll.live.as_ref().unwrap();
        assert!(!live.ok);
        assert!(live.summary.contains("unreachable"), "{}", live.summary);
        assert!(text(&poll.detail_lines()).contains("no route to gate"));
    }

    #[test]
    fn an_unfetched_row_says_what_enter_does() {
        let poll = row(&record()).unwrap();
        assert!(poll.live.is_none());
        assert!(text(&poll.detail_lines()).contains("r asks the gate"));
    }

    #[test]
    fn the_resolution_is_optional_and_the_input_says_so() {
        let input = ResolutionInput {
            poll_id: "psyc60-mid".into(),
            buffer: String::new(),
        };
        assert!(input.title().contains("optional"), "{}", input.title());
    }
}
