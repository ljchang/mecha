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
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

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
    /// The thread being read in full. Built by `mecha mail show` in a child
    /// process, off the event loop — see `spawn_mail_read`.
    pub reading: Option<Reader>,
    /// A read in flight, by handle. A second `enter` while one is loading
    /// would stack a second MCP startup on an impatient keypress.
    pub loading: Option<String>,
    /// The key list, on `?`.
    pub help: bool,
}

/// One thread, open and readable.
///
/// A viewer and nothing else: the text is whatever `mecha mail show` printed,
/// so there is exactly one renderer of a thread and the modal cannot drift
/// from what the command line says. The same rule the rest of this module
/// follows for mutations, applied to reading.
pub struct Reader {
    /// The thread's handle, for the title.
    pub handle: String,
    pub lines: Vec<String>,
    pub scroll: u16,
}

impl Reader {
    pub fn new(handle: String, text: &str) -> Self {
        Reader {
            handle,
            lines: text.lines().map(str::to_string).collect(),
            scroll: 0,
        }
    }

    /// Scroll, stopping at both ends. Running off the bottom into blank space
    /// reads as a truncated message rather than as the end of one.
    pub fn scroll_by(&mut self, delta: i16) {
        let max = self.lines.len().saturating_sub(1) as u16;
        self.scroll = self.scroll.saturating_add_signed(delta).min(max);
    }
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
            reading: None,
            loading: None,
            help: false,
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

/// One key, and what it is called on each of the two surfaces that name it.
pub struct Key {
    pub key: char,
    /// The verb, for the strip across the top of the list.
    pub short: &'static str,
    /// What it actually does, for `?`.
    pub note: &'static str,
}

/// Every key this modal answers to, written down once.
///
/// **A modal whose actions are invisible is a modal with one action** — the
/// `/outbox` note, one step on. That title carried its four keys comfortably;
/// eleven do not fit in a title on any terminal, and the answer to "they do
/// not fit" was, for a while, to show none of them. So the strip across the
/// top of the list names the verbs and `?` explains them: the same two tiers
/// of progressive disclosure the main help overlay already uses.
///
/// One table, because two would drift — a legend that advertises a key the
/// map does not answer to is worse than no legend, and a test asserts the two
/// agree. `enter` and the movement keys are here too: they are keys a person
/// needs, and "documented only where the code happens to mention them" is how
/// `enter` went a whole release doing nothing visible.
pub const KEYS: &[Key] = &[
    Key {
        key: 'j',
        short: "",
        note: "move down (↓ too)",
    },
    Key {
        key: 'k',
        short: "",
        note: "move up (↑ too)",
    },
    Key {
        key: '\n',
        short: "enter read",
        note: "read the whole thread, in full",
    },
    Key {
        key: 'a',
        short: "a archive",
        note: "archive it — reversible, and nobody else learns anything",
    },
    Key {
        key: 's',
        short: "s spam",
        note: "mark spam — asks first: it trains the provider's filter",
    },
    Key {
        key: 't',
        short: "t task",
        note: "turn it into a task",
    },
    Key {
        key: 'd',
        short: "d dismiss",
        note: "dismiss it — the mailbox is left alone",
    },
    Key {
        key: 'n',
        short: "n needs-info",
        note: "park it until someone answers a question",
    },
    Key {
        key: '!',
        short: "! wrong",
        note: "correct the classifier's bucket",
    },
    Key {
        key: 'r',
        short: "r reply",
        note: "draft a reply — it lands in /outbox for review",
    },
    Key {
        key: 'f',
        short: "f forward",
        note: "forward it — lands in /outbox",
    },
    Key {
        key: 'e',
        short: "e schedule",
        note: "draft a calendar reply — lands in /outbox",
    },
    Key {
        key: '?',
        short: "",
        note: "this list",
    },
    Key {
        key: 'q',
        short: "",
        note: "close (esc too)",
    },
];

/// The strip across the top of the list: the verbs, in key order, skipping
/// the ones the title and the shape of a list already teach.
pub fn key_strip() -> String {
    KEYS.iter()
        .filter(|k| !k.short.is_empty())
        .map(|k| k.short)
        .collect::<Vec<_>>()
        .join(" · ")
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
        if let Some(handle) = &self.loading {
            return format!(" mail — reading {handle}… ");
        }
        let (need, parked) = self.counts();
        format!(" mail — {need} need you · {parked} parked · ? keys · esc ")
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
        if self.help {
            self.draw_help(frame);
            return;
        }
        if let Some(reader) = &self.reading {
            draw_reader(frame, reader);
            return;
        }
        if let Some(input) = &self.input {
            if input.contacts.is_empty() {
                super::outbox::draw_reason_input(frame, &input.title(), &input.buffer);
            } else {
                draw_recipient_input(frame, input);
            }
            return;
        }
        // The keys, across the top and above the list. A strip rather than a
        // longer title: eleven of them do not fit in a border, and a title
        // that truncates teaches whichever half the terminal happened to fit.
        let strip = Line::styled(format!("  {}", key_strip()), Style::new().fg(Color::Cyan));
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
                    // **No handle column.** It is a thread id's last eight
                    // characters — the join to the CLI, and eight characters
                    // of noise to someone who selects with a cursor. It is on
                    // the reader's title bar, which is where a person who
                    // wants to type `mecha mail archive <handle>` is looking.
                    let text = format!(
                        "{marker} {bullet} {:<7} {:<18} {:<58} {}",
                        row.urgency,
                        truncate(&row.tags, 18),
                        truncate(&row.summary, 58),
                        truncate(&row.from, 26),
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

        let height = (body.len() as u16 + 1).clamp(2, frame.area().height.saturating_sub(4)) + 2;
        let area = super::centered(frame.area(), 120, height);
        frame.render_widget(Clear, area);
        // The strip is rendered *outside* the scrolling paragraph, in the
        // first line of the block. Inside it, a queue longer than the box
        // scrolls the legend away — and a legend that disappears exactly when
        // the list gets big enough to need one is not a legend.
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(Color::Cyan))
            .title(self.title());
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height == 0 {
            return;
        }
        frame.render_widget(Paragraph::new(strip), Rect { height: 1, ..inner });
        let list = Rect {
            y: inner.y + 1,
            height: inner.height.saturating_sub(1),
            ..inner
        };
        frame.render_widget(
            Paragraph::new(body).scroll((self.list_scroll(list.height), 0)),
            list,
        );
    }

    /// `?` — every key, with what it does.
    fn draw_help(&self, frame: &mut Frame) {
        let body: Vec<Line> = KEYS
            .iter()
            .map(|k| {
                let key = if k.key == '\n' {
                    "enter".to_string()
                } else {
                    k.key.to_string()
                };
                Line::from(vec![
                    Span::styled(format!("  {key:<8}"), Style::new().fg(Color::Cyan)),
                    Span::styled(k.note, Style::new().fg(Color::White)),
                ])
            })
            .collect();
        let area = super::centered(frame.area(), 74, body.len() as u16 + 2);
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(body).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(Color::Cyan))
                    .title(" mail keys · any key closes "),
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

/// The thread, open.
///
/// **`enter` used to run `mecha mail show` and print its `subject:` line into
/// the title bar** — a whole mail fetch, an MCP server started, a thread
/// downloaded, and one line of it shown. The record was fetched and thrown
/// away. Everything below the header is what a person opened it for.
///
/// The header lines the command prints — account, from, subject, date, and
/// the classifier's own reasoning — are greyed and the message is not, so the
/// eye lands on the prose rather than on the metadata above it.
fn draw_reader(frame: &mut Frame, reader: &Reader) {
    let grey = Style::new().fg(Color::DarkGray);
    let body: Vec<Line> = reader
        .lines
        .iter()
        .map(|line| {
            let meta = [
                "account:",
                "from:",
                "subject:",
                "date:",
                "verdict:",
                "tags:",
                "reasoning:",
                "looks like",
            ]
            .iter()
            .any(|p| line.starts_with(p));
            if meta {
                Line::styled(line.clone(), grey)
            } else {
                Line::styled(line.clone(), Style::new().fg(Color::White))
            }
        })
        .collect();
    let area = super::centered(frame.area(), 100, frame.area().height.saturating_sub(4));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: false })
            .scroll((reader.scroll, 0))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(Color::Cyan))
                    .title(format!(
                        " {} · ↑↓ scroll · r reply · a archive · ? keys · esc back ",
                        reader.handle
                    )),
            ),
        area,
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One table, checked both ways. A legend advertising a key the map does
    /// not answer to is worse than no legend — it teaches a keypress that
    /// does nothing and reads as a broken modal — and a key the legend omits
    /// is a feature nobody finds, which is how `enter` came to be a whole
    /// mail fetch printing one line into a title bar.
    #[test]
    fn the_legend_and_the_key_map_are_the_same_set() {
        // Every advertised *action* exists. The entries with no verb — move,
        // read, help, close — are the modal's own keys rather than the map's,
        // which is exactly why they carry no verb.
        // `enter` is the exception: it carries a verb because it is worth
        // advertising, and it is the modal's own key because opening a reader
        // is not a `mecha mail` verb.
        for key in KEYS.iter().filter(|k| !k.short.is_empty() && k.key != '\n') {
            assert!(
                action_for(key.key).is_some(),
                "the strip advertises `{}`, which the map does not answer to",
                key.key
            );
        }
        for c in ' '..='~' {
            if let Some(action) = action_for(c) {
                assert!(
                    KEYS.iter().any(|k| k.key == c),
                    "`{c}` runs {action:?} and is in no legend",
                );
            }
        }
    }

    /// The strip is the verbs, not every key: `j`/`k`/`?`/`q` are taught by
    /// the title and by the shape of a list, and spending the width on them
    /// would push a real action off the end.
    #[test]
    fn the_key_strip_names_the_actions_and_fits_a_line() {
        let strip = key_strip();
        for expected in ["enter read", "a archive", "s spam", "r reply"] {
            assert!(strip.contains(expected), "{expected} missing from {strip}");
        }
        assert!(!strip.contains(" j "), "{strip}");
        assert!(strip.chars().count() < 116, "{} wide: {strip}", strip.len());
    }

    fn row(summary: &str) -> MailRow {
        MailRow {
            thread_id: "AAQkADFiNjVjOWI1LTlkNGEtNDcxMi00ZDVmLWM3ZWI=".into(),
            handle: handle("AAQkADFiNjVjOWI1LTlkNGEtNDcxMi00ZDVmLWM3ZWI="),
            account: "dartmouth".into(),
            urgency: "week".into(),
            tags: "#research".into(),
            summary: summary.into(),
            from: "someone@example.org".into(),
            state: CLASSIFIED.into(),
            needs_me: true,
        }
    }

    /// The reader stops at the last line. Scrolling past the end shows blank
    /// space, which reads as a message that was cut off rather than one that
    /// ended.
    #[test]
    fn the_reader_scrolls_within_its_own_text() {
        let mut reader = Reader::new("ubwPPLw=".into(), "one\ntwo\nthree");
        reader.scroll_by(-5);
        assert_eq!(reader.scroll, 0);
        reader.scroll_by(50);
        assert_eq!(reader.scroll, 2);
    }

    /// The list is for recognising a thread, and eight characters of base64
    /// help nobody do that. The handle is kept on the row — the reader's
    /// title shows it, and it is what `mecha mail archive <handle>` takes.
    #[test]
    fn the_list_spends_its_width_on_the_subject_not_the_id() {
        let modal = MailModal::new(vec![row("Peer review request for manuscript 2026-25921")]);
        let mut buffer = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::new(0, 0, 130, 20));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(130, 20)).unwrap();
        terminal.draw(|f| modal.draw(f)).unwrap();
        buffer.clone_from(terminal.backend().buffer());
        let screen: String = buffer
            .content()
            .chunks(130)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !screen.contains(&modal.rows[0].handle),
            "the handle is noise in a list you select with a cursor:\n{screen}"
        );
        assert!(
            screen.contains("Peer review request for manuscript 2026-25921"),
            "the subject survives the width it freed:\n{screen}"
        );
        // The legend is on screen without being asked for.
        assert!(screen.contains("a archive"), "{screen}");
    }
}
