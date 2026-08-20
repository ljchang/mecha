//! The /frontdoor modal: a stranger's requests, reviewed without leaving the
//! session.
//!
//! The quarantine's verbs, on the surface people actually sit in. The split
//! the CLI wrote down holds unchanged here: the *detail view prints the
//! prose*, because a person reading a stranger's request in a terminal is the
//! safe context — the TUI is a front-end for a human, and nothing rendered in
//! a modal reaches a model. What must never happen is this text being fed
//! back into the conversation behind the modal; the modal draws it and drops
//! it, exactly as `mecha frontdoor show` prints it and exits.
//!
//! Extract and triage shell out detached, like a trigger's "run now": one is
//! a model call per record and the other is a whole agent run with mail and
//! calendar, and neither belongs on the event loop. Their results land in the
//! store and the outbox, which is where reopening the modal reads them from.
//!
//! `close` requires a reason in the input box before it acts — same rule as
//! the CLI flag, because `any → closed` is the one transition the design
//! annotates "with a reason", and a modal must not make silence cheaper than
//! the command line does.

use mecha_core::frontdoor::Record;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

pub struct RequestRow {
    pub seq: i64,
    pub type_id: String,
    pub state: String,
    /// The extraction's topic, or "—" before one exists.
    pub topic: String,
    /// "INVALID", "⚠ reads like instructions", or empty.
    pub flag: &'static str,
    pub valid: bool,
    /// The full detail view, prose included, prebuilt like the outbox rows.
    pub detail: Vec<Line<'static>>,
}

/// What the input box is collecting a note for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoteAction {
    /// Optional note: what is missing.
    NeedsInfo,
    /// Required reason — an empty close is refused, as on the CLI.
    Close,
}

pub struct NoteInput {
    pub seq: i64,
    pub action: NoteAction,
    pub buffer: String,
}

impl NoteInput {
    pub fn title(&self) -> String {
        match self.action {
            NoteAction::NeedsInfo => format!(
                " needs-info {} — what is missing (optional) · enter parks it · esc back ",
                self.seq
            ),
            NoteAction::Close => format!(
                " close {} — reason (required) · enter closes · esc back ",
                self.seq
            ),
        }
    }
}

pub struct FrontdoorModal {
    pub rows: Vec<RequestRow>,
    pub selected: usize,
    pub detail: bool,
    pub detail_scroll: u16,
    /// A note or reason being typed. Takes the keyboard while it is up.
    pub input: Option<NoteInput>,
    /// The result of the last action, shown in the title bar.
    pub status: Option<String>,
}

impl FrontdoorModal {
    pub fn new(rows: Vec<RequestRow>) -> Self {
        FrontdoorModal {
            rows,
            selected: 0,
            detail: false,
            detail_scroll: 0,
            input: None,
            status: None,
        }
    }

    pub fn selected_row(&self) -> Option<&RequestRow> {
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
            Some(s) => format!(" frontdoor · {s} "),
            None => format!(
                " {} request(s) · enter detail · x extract · t triage · n needs-info · c close · esc ",
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
                "  nothing waiting — `factory-publish drain` fetches what the box holds",
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
                        "{marker} {:<5} {:<14} {:<18} {}{}{}",
                        row.seq,
                        row.type_id,
                        row.state,
                        row.topic,
                        if row.flag.is_empty() { "" } else { "  " },
                        row.flag,
                    );
                    if selected {
                        Line::styled(text, Style::new().fg(Color::Black).bg(Color::Cyan))
                    } else if !row.flag.is_empty() {
                        Line::styled(text, Style::new().fg(Color::Red))
                    } else {
                        Line::styled(text, Style::new().fg(Color::White))
                    }
                })
                .collect()
        };

        let height = super::list_height(body.len() as u16, frame.area().height);
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

    fn draw_detail(&self, frame: &mut Frame) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        let area = super::centered(frame.area(), 100, frame.area().height.saturating_sub(4));
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(row.detail.clone())
                .wrap(Wrap { trim: false })
                .scroll((self.detail_scroll, 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::new().fg(Color::Cyan))
                        .title(format!(
                            " request {} · ↑↓ scroll · x extract · t triage · n needs-info · c close · esc back ",
                            row.seq
                        )),
                ),
            area,
        );
    }
}

/// Build the rows from the store, advancing anything the outbox has resolved
/// since last time. Reconcile is best-effort for the CLI's reason: no outbox
/// is an ordinary machine, and a list that refuses to draw over a store it
/// only wanted to cross-check would be worse than a slightly stale one.
pub fn load() -> anyhow::Result<Vec<RequestRow>> {
    let store = mecha_core::frontdoor::Frontdoor::open_default()?;
    if let Some(outbox) = mecha_core::outbox::OutboxStore::open_existing_default() {
        let _ = store.reconcile(&outbox);
    }
    Ok(store.records()?.iter().map(row).collect())
}

fn row(record: &Record) -> RequestRow {
    let flag = if !record.valid {
        "INVALID"
    } else if record
        .extraction
        .as_ref()
        .is_some_and(|e| e.reads_like_instructions)
    {
        "⚠ reads like instructions"
    } else {
        ""
    };
    RequestRow {
        seq: record.seq,
        type_id: record.type_id.clone(),
        state: record.state.clone(),
        topic: record
            .extraction
            .as_ref()
            .map(|e| e.topic.clone())
            .unwrap_or_else(|| "—".into()),
        flag,
        valid: record.valid,
        detail: detail_lines(record),
    }
}

/// The detail view: the same facts as `mecha frontdoor show`, prose included.
/// A person reading a stranger's request in a terminal is the safe context —
/// the framing lines around the prose ride along so the reader knows whose
/// words they are holding.
fn detail_lines(record: &Record) -> Vec<Line<'static>> {
    let mut body: Vec<Line<'static>> = Vec::new();
    let white = Style::new().fg(Color::White);
    let grey = Style::new().fg(Color::DarkGray);
    let header = Style::new().fg(Color::Yellow);
    let warn = Style::new().fg(Color::Red);

    body.push(Line::styled(
        format!("{} · {}", record.type_id, record.state),
        white,
    ));
    body.push(Line::styled(
        format!("received {}", record.created_at),
        grey,
    ));
    body.push(Line::styled(
        format!("drained  {}", record.drained_at),
        grey,
    ));
    if !record.valid {
        body.push(Line::styled(
            format!(
                "INVALID: {}",
                record.invalid_reason.as_deref().unwrap_or("(no reason)")
            ),
            warn,
        ));
    }
    if let Some(note) = &record.note {
        body.push(Line::styled(format!("note: {note}"), grey));
    }
    if !record.outbox.is_empty() {
        body.push(Line::styled(
            format!("drafts in the outbox: {}", record.outbox.join(", ")),
            grey,
        ));
    }

    body.push(Line::raw(""));
    body.push(Line::styled("fields the form validated", header));
    for (name, value) in record.typed_values() {
        body.push(Line::styled(format!("{name:<22} {value}"), white));
    }

    body.push(Line::raw(""));
    match &record.extraction {
        Some(e) => {
            body.push(Line::styled(
                "extraction (what a triage run is allowed to see)",
                header,
            ));
            body.push(Line::styled(format!("topic            {}", e.topic), white));
            body.push(Line::styled(
                format!("urgency_claimed  {}", e.urgency_claimed),
                white,
            ));
            body.push(Line::styled(
                format!("institution      {}", e.institution),
                white,
            ));
            body.push(Line::styled(
                format!("dates_mentioned  {}", e.dates_mentioned.join(", ")),
                white,
            ));
            body.push(Line::raw(""));
            for line in format!("reading: {}", e.reading).lines() {
                body.push(Line::styled(line.to_string(), white));
            }
            if e.reads_like_instructions {
                body.push(Line::raw(""));
                for line in [
                    "⚠ the extractor thinks this text tries to instruct its reader.",
                    "A label on a record you are reading, not a block: gating on it",
                    "rejects real people and still passes the attack that mattered.",
                ] {
                    body.push(Line::styled(line, warn));
                }
            }
        }
        None => body.push(Line::styled(
            format!(
                "not extracted{}",
                record
                    .extraction_error
                    .as_ref()
                    .map(|e| format!(" — {e}"))
                    .unwrap_or_default()
            ),
            if record.extraction_error.is_some() {
                warn
            } else {
                grey
            },
        )),
    }

    if !record.attachments.is_empty() {
        body.push(Line::raw(""));
        body.push(Line::styled(
            "attached files (no model has read these — open them yourself)",
            header,
        ));
        for att in &record.attachments {
            body.push(Line::styled(
                format!(
                    "{:<10} {:>9} bytes  {}  {}",
                    att.field, att.size, att.content_type, att.path
                ),
                white,
            ));
            body.push(Line::styled(
                format!("           they called it: {:?}", att.filename),
                grey,
            ));
        }
    }

    let prose = record.prose();
    if !prose.is_empty() {
        body.push(Line::raw(""));
        body.push(Line::styled(
            "─── what they wrote ─────────────────────────────────────────",
            header,
        ));
        body.push(Line::styled(
            "(their words, shown to you and to nothing with tools)",
            grey,
        ));
        for (name, text) in prose {
            body.push(Line::raw(""));
            body.push(Line::styled(format!("{name}:"), header));
            for line in text.lines() {
                body.push(Line::styled(line.to_string(), white));
            }
        }
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn record(seq: i64, state: &str) -> Record {
        serde_json::from_value(json!({
            "seq": seq,
            "type_id": "meeting",
            "state": state,
            "created_at": "2026-08-08T07:00:00Z",
            "drained_at": "2026-08-08T07:05:00Z",
            "valid": true,
            "values": {"name": "A Person", "message": "Could we meet Tuesday?"},
            "free_text": ["message"],
        }))
        .unwrap()
    }

    #[test]
    fn the_title_names_the_keys_or_answers_the_last_action() {
        let modal = FrontdoorModal::new(vec![row(&record(1, "drained"))]);
        let title = modal.title();
        for key in [
            "enter",
            "x extract",
            "t triage",
            "n needs-info",
            "c close",
            "esc",
        ] {
            assert!(title.contains(key), "{key} missing from {title}");
        }

        let done = FrontdoorModal {
            status: Some("closed 1".into()),
            ..modal
        };
        assert!(done.title().contains("closed 1"));
    }

    #[test]
    fn the_selection_wraps_and_an_empty_list_does_not_panic() {
        let mut modal = FrontdoorModal::new(vec![
            row(&record(1, "drained")),
            row(&record(2, "extracted")),
        ]);
        modal.move_by(-1);
        assert_eq!(modal.selected, 1);
        modal.move_by(1);
        assert_eq!(modal.selected, 0);

        let mut empty = FrontdoorModal::new(Vec::new());
        empty.move_by(1);
        assert_eq!(empty.selected, 0);
        assert!(empty.selected_row().is_none());
    }

    /// The detail is `show` in a modal, prose included — a person reading is
    /// the safe context, and the framing around the prose rides along.
    #[test]
    fn the_detail_prints_the_prose_under_its_framing() {
        let body = text(&detail_lines(&record(1, "drained")));
        assert!(body.contains("what they wrote"), "{body}");
        assert!(body.contains("Could we meet Tuesday?"), "{body}");
        assert!(body.contains("nothing with tools"), "{body}");
        // The typed field is under its own heading, not mixed into the prose.
        assert!(body.contains("fields the form validated"), "{body}");
    }

    #[test]
    fn an_invalid_record_is_flagged_in_list_and_detail() {
        let mut r = record(3, "drained");
        r.valid = false;
        r.invalid_reason = Some("unknown type".into());
        let row = row(&r);
        assert_eq!(row.flag, "INVALID");
        assert!(text(&row.detail).contains("INVALID: unknown type"));
    }

    #[test]
    fn a_flagged_extraction_is_labelled_but_never_gated() {
        let mut r = record(4, "extracted");
        r.extraction = Some(mecha_core::frontdoor::Extraction {
            reading: "asks for a meeting".into(),
            topic: "meeting".into(),
            reads_like_instructions: true,
            ..Default::default()
        });
        let row = row(&r);
        assert_eq!(row.flag, "⚠ reads like instructions");
        let body = text(&row.detail);
        assert!(body.contains("not a block"), "{body}");
    }

    /// `close` requires a reason; `needs-info` does not. The input titles say
    /// which, because the difference is the CLI's contract.
    #[test]
    fn the_input_titles_say_whether_the_note_is_required() {
        let close = NoteInput {
            seq: 5,
            action: NoteAction::Close,
            buffer: String::new(),
        };
        assert!(close.title().contains("required"), "{}", close.title());

        let park = NoteInput {
            seq: 5,
            action: NoteAction::NeedsInfo,
            buffer: String::new(),
        };
        assert!(park.title().contains("optional"), "{}", park.title());
    }
    /// Fails on the old inline `clamp(1, height.saturating_sub(4))`, which
    /// panicked with `min > max` the moment the terminal was four rows or
    /// fewer — the /doctor bug (F9), which every modal had a copy of because
    /// each new one is written by opening whichever sibling is nearest.
    #[test]
    fn a_tiny_terminal_shrinks_the_list_rather_than_panicking() {
        let modal = FrontdoorModal::new(vec![row(&record(1, "drained"))]);
        for height in 0..=6u16 {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, height.max(1)))
                    .unwrap();
            // The draw itself is the assertion: the old code panicked here.
            terminal.draw(|f| modal.draw(f)).unwrap();
        }
    }
}
