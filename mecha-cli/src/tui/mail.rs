//! The `/mail` modal — working the triage queue without leaving the TUI.
//!
//! Sixth modal on the `/outbox` pattern, and it inherits that pattern's whole
//! shape deliberately: **the store is read for display, and every mutation is a
//! `mecha mail …` child process.** Nothing here reimplements a verb.
//!
//! That is not tidiness. The store and the CLI are the product and every
//! front-end is one reader — the nightly, the morning briefing and this modal
//! all act through the same commands, so a thing the modal can do is a thing a
//! script can do, and a modal-only action would be a feature no trigger could
//! ever use. `MAIL-UX-DESIGN.md` §5.
//!
//! Two rules carried across from `/outbox`:
//!
//! - **Slow work spawns detached and is watched by polling the store**, never
//!   the child. A reply builds a whole tool surface and can take minutes;
//!   doing it on the event loop freezes the interface.
//! - **The result of a reply lands in `/outbox`, not here.** There is exactly
//!   one approval surface and this is not it. `/mail` decides *whether*
//!   something needs an answer; `/outbox` decides whether *this* answer goes.
//!
//! Unlike the front door, this modal shows prose. That is the same reasoning
//! `frontdoor show` uses: a person reading their own mail in a terminal is the
//! safe context, and a list nobody can recognise a thread in is not a list.
//! What must not see the prose is a *privileged run*, and none happens here.

use anyhow::Result;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use mecha_core::mail_triage::{
    contact_candidates, handle, recipient_token, Bucket, Contact, Record, TriageStore, CLASSIFIED,
    FAILED, PARKED,
};

/// One thread, flattened for display.
pub struct MailRow {
    pub thread_id: String,
    pub handle: String,
    pub account: String,
    pub urgency: String,
    pub tags: String,
    pub summary: String,
    pub from: String,
    pub state: String,
    pub needs_me: bool,
}

pub struct MailModal {
    pub rows: Vec<MailRow>,
    pub selected: usize,
    /// A reason being typed — a needs-info note or a correction. Takes the
    /// keyboard while it is up.
    pub input: Option<MailInput>,
    /// Confirmation pending for an action that reaches outside the mailbox.
    pub confirm: Option<String>,
    pub status: Option<String>,
}

pub struct MailInput {
    pub label: String,
    pub verb: &'static str,
    pub buffer: String,
    /// Byte offset, so completion can apply to the recipient under the cursor
    /// rather than the last one typed.
    pub cursor: usize,
    /// Recipient completion. Empty for the inputs that are free text — a
    /// needs-info reason has no candidates and should not pretend to.
    pub contacts: Vec<Contact>,
    pub pick: usize,
}

impl MailInput {
    pub fn text(label: &str, verb: &'static str) -> Self {
        MailInput {
            label: label.into(),
            verb,
            buffer: String::new(),
            cursor: 0,
            contacts: Vec::new(),
            pick: 0,
        }
    }

    pub fn recipients(label: &str, verb: &'static str, contacts: Vec<Contact>) -> Self {
        MailInput {
            label: label.into(),
            verb,
            buffer: String::new(),
            cursor: 0,
            contacts,
            pick: 0,
        }
    }

    /// Candidates for the recipient under the cursor.
    pub fn candidates(&self) -> Vec<&Contact> {
        if self.contacts.is_empty() {
            return Vec::new();
        }
        let (_, partial) = recipient_token(&self.buffer, self.cursor);
        contact_candidates(partial, &self.contacts, 6)
    }

    /// Replace the recipient under the cursor with a chosen address.
    ///
    /// Leaves `", "` after it so the next one can be typed straight away —
    /// the whole point of the key is that several recipients are normal.
    pub fn accept(&mut self, address: &str) {
        let (start, partial) = recipient_token(&self.buffer, self.cursor);
        let end = start
            + self.buffer[start..].len().min(
                self.buffer[start..]
                    .find(',')
                    .unwrap_or(self.buffer.len() - start),
            );
        let lead = if start == 0 { "" } else { " " };
        let replaced = format!("{lead}{address}, ");
        let _ = partial;
        self.buffer.replace_range(start..end, &replaced);
        self.cursor = start + replaced.len();
        self.pick = 0;
    }

    pub fn insert(&mut self, c: char) {
        self.buffer.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.pick = 0;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.buffer[..self.cursor]
            .chars()
            .next_back()
            .map(char::len_utf8)
            .unwrap_or(1);
        self.cursor -= prev;
        self.buffer.remove(self.cursor);
        self.pick = 0;
    }
}

impl MailModal {
    pub fn new(rows: Vec<MailRow>) -> Self {
        MailModal {
            rows,
            selected: 0,
            input: None,
            confirm: None,
            status: None,
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as isize;
        self.selected = ((self.selected as isize + delta).rem_euclid(n)) as usize;
    }

    /// The title bar's counts. Read from the rows rather than recomputed by a
    /// second query, so what the header claims and what the list shows cannot
    /// disagree.
    pub fn counts(&self) -> (usize, usize) {
        (
            self.rows.iter().filter(|r| r.needs_me).count(),
            self.rows.iter().filter(|r| r.state == PARKED).count(),
        )
    }
}

/// Read the store for display.
pub fn load() -> Result<Vec<MailRow>> {
    let Some(store) = TriageStore::open_existing_default() else {
        return Ok(Vec::new());
    };
    let mut rows: Vec<MailRow> = store.list()?.iter().map(row).collect();
    // Respond first, then by urgency, then newest — the order a person works
    // in, not the order the filesystem hands back.
    rows.sort_by_key(|r| {
        (
            !r.needs_me,
            match r.urgency.as_str() {
                "now" => 0,
                "today" => 1,
                "week" => 2,
                _ => 3,
            },
        )
    });
    Ok(rows)
}

fn row(r: &Record) -> MailRow {
    let v = r.verdict.as_ref();
    MailRow {
        thread_id: r.thread_id.clone(),
        handle: handle(&r.thread_id),
        account: r.account.clone(),
        urgency: v
            .map(|v| v.urgency.as_str().to_string())
            .unwrap_or_default(),
        tags: v
            .map(|v| {
                v.tags
                    .iter()
                    .map(|t| format!("#{t}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default(),
        summary: v
            .map(|v| v.one_line.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| r.subject.clone()),
        from: r.from.clone(),
        state: r.state.clone(),
        needs_me: r.state == CLASSIFIED && v.is_some_and(|v| v.bucket == Bucket::Respond)
            || r.state == FAILED,
    }
}

/// What a key means. Returned rather than executed so the key map is a value
/// the tests can read — the modal's whole action set in one place.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Runs now: a single call, no model.
    Now(&'static str),
    /// Runs now, but asks first — it reaches outside the mailbox.
    Confirm(&'static str),
    /// Wants text before it can run.
    Prompt(&'static str, &'static str),
    /// A whole agent run: spawns detached, watched by polling the store.
    Detached(&'static str),
    /// Asks who to send to, with completion, then runs detached.
    Recipients(&'static str),
    Close,
}

/// The key map.
///
/// **`s` confirms and the others do not**, which is the one asymmetry worth
/// stating: spam trains the provider's filter, so it is the only triage action
/// with an effect outside the user's own mailbox. Archive is reversible and
/// private; spam is neither.
pub fn action_for(key: char) -> Option<Action> {
    Some(match key {
        'a' => Action::Now("archive"),
        's' => Action::Confirm("spam"),
        't' => Action::Now("task"),
        'd' => Action::Now("dismiss"),
        'n' => Action::Prompt("needs-info", "waiting for"),
        '!' => Action::Prompt("correct", "should have been"),
        'r' => Action::Detached("reply"),
        'f' => Action::Recipients("forward"),
        'e' => Action::Detached("schedule"),
        'q' => Action::Close,
        _ => return None,
    })
}

impl MailInput {
    pub fn title(&self) -> String {
        format!(" {} — enter to confirm, esc to cancel ", self.label)
    }
}

impl MailModal {
    fn title(&self) -> String {
        if let Some(status) = &self.status {
            return format!(" mail — {status} ");
        }
        if let Some(confirm) = &self.confirm {
            return format!(" {confirm} ");
        }
        let (need, parked) = self.counts();
        format!(
            " mail — {need} need you · {parked} parked · a archive · s spam · t task · n needs-info · ! wrong · esc "
        )
    }

    /// Keep the selection on screen.
    ///
    /// **Without this the highlight walks off the bottom and keeps going** —
    /// `load()` returns every record in the store, which grows nightly, so a
    /// person pressing `j` past the last visible row would archive or spam a
    /// thread they cannot see. Same helper the sibling modals use.
    fn list_scroll(&self, visible: u16) -> u16 {
        let visible = visible.max(1) as usize;
        (self.selected + 1).saturating_sub(visible) as u16
    }

    pub fn draw(&self, frame: &mut Frame) {
        if let Some(input) = &self.input {
            if input.contacts.is_empty() {
                super::outbox::draw_reason_input(frame, &input.title(), &input.buffer);
            } else {
                draw_recipient_input(frame, input);
            }
            return;
        }
        let body: Vec<Line> = if self.rows.is_empty() {
            vec![Line::styled(
                "  nothing classified yet — `mecha mail classify` fills the queue",
                Style::new().fg(Color::DarkGray),
            )]
        } else {
            self.rows
                .iter()
                .enumerate()
                .map(|(i, row)| {
                    let selected = i == self.selected;
                    let marker = if selected { "›" } else { " " };
                    // The `●` is the same mark `mecha mail list` uses for a
                    // thread that needs an answer: one vocabulary across every
                    // reader of this store.
                    let bullet = if row.needs_me { "●" } else { " " };
                    let text = format!(
                        "{marker} {bullet} {:<7} {:<9} {:<18} {:<52} {}",
                        row.urgency,
                        row.handle,
                        truncate(&row.tags, 18),
                        truncate(&row.summary, 52),
                        truncate(&row.from, 28),
                    );
                    if selected {
                        Line::styled(text, Style::new().fg(Color::Black).bg(Color::Cyan))
                    } else if row.state == FAILED {
                        Line::styled(text, Style::new().fg(Color::Red))
                    } else if row.state == PARKED {
                        Line::styled(text, Style::new().fg(Color::DarkGray))
                    } else if row.needs_me {
                        Line::styled(text, Style::new().fg(Color::White))
                    } else {
                        Line::styled(text, Style::new().fg(Color::DarkGray))
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
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    // Says it was cut, on the mecha-slack rule: a builder that truncates
    // invisibly leaves a reader with a complete-looking line missing the part
    // that mattered.
    format!(
        "{}…",
        s.chars().take(n.saturating_sub(1)).collect::<String>()
    )
}

/// The recipient line, with the candidates under it.
///
/// The menu is always shown rather than appearing once something is typed: a
/// completion that only reveals itself after a correct guess helps whoever
/// already knew the answer. An empty partial lists the people who write most.
fn draw_recipient_input(frame: &mut Frame, input: &MailInput) {
    let candidates = input.candidates();
    let mut body = vec![
        Line::styled(format!("{}▏", input.buffer), Style::new().fg(Color::White)),
        Line::styled("", Style::new()),
    ];
    if candidates.is_empty() {
        body.push(Line::styled(
            "  no match — type a whole address to use one anyway",
            Style::new().fg(Color::DarkGray),
        ));
    }
    for (i, c) in candidates.iter().enumerate() {
        let text = format!(
            "  {} {:<38} {}",
            if i == input.pick { "›" } else { " " },
            c.address,
            c.name
        );
        body.push(if i == input.pick {
            Line::styled(text, Style::new().fg(Color::Black).bg(Color::Cyan))
        } else {
            Line::styled(text, Style::new().fg(Color::White))
        });
    }
    let height = body.len() as u16 + 2;
    let area = super::centered(frame.area(), 90, height);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(body).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Yellow))
                .title(format!(
                    " {} — tab completes · ↑↓ picks · comma for another · enter sends to /outbox ",
                    input.label
                )),
        ),
        area,
    );
}
