//! The /tasks modal: the GTD board in the knowledge graph — what is on it,
//! capture, and moving a task through its lifecycle, without leaving the
//! session.
//!
//! The `/mail` pattern, and the `/triggers` rule underneath it: **every
//! mutation drives `mecha tasks …` as a child process**, so there is one
//! implementation per verb and no way for this modal to do something the
//! command line cannot. What it reads is `mecha tasks list --json`, which is
//! the same JSON `kg_task_list` hands the model — three readers of one store,
//! none of them holding a second copy of it.
//!
//! **Nothing here confirms, which is the one asymmetry worth stating.** `/mail`
//! confirms on `s` because marking spam trains the provider's filter and so
//! reaches outside the user's own mailbox. The board reaches nobody:
//! `kg_task_*` is `openWorldHint: false`, every status is a status away from
//! where it was, and there is no delete on the tool surface at all. A
//! confirmation for a reversible, private change is a keystroke that teaches
//! people to hit enter without reading.
//!
//! The board is drawn and dropped: nothing rendered in a modal reaches a
//! model, so a task's own words stay on the human's side of the boundary.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

/// One row of the board, as `kg_task_list` reports it.
pub struct TaskRow {
    /// The node id — the join to `mecha tasks set`. Kept off the list and
    /// shown in the detail, on the `/mail` finding: eight characters of id
    /// help nobody recognise a task in a list they select with a cursor.
    pub id: String,
    pub name: String,
    pub status: String,
    pub due_at: Option<String>,
    pub defer_until: Option<String>,
    pub context: Option<String>,
    pub project: Option<String>,
    pub waiting_on: Option<String>,
    /// The *graph's* verdict, not one computed here — it holds the clock and
    /// the completion stamp, and a second definition of "late" would drift.
    pub overdue: bool,
    pub closed: bool,
}

/// The four statuses a task can be in while it is still work. `done` and
/// `dropped` are the other two, and they are an ending rather than a step,
/// which is why `space` walks these and not those.
pub const ACTIVE_STATUSES: [&str; 4] = ["next", "inbox", "scheduled", "waiting"];

/// A form being filled in: a capture when `editing` is `None`, otherwise the
/// schedule of that task id.
///
/// **There is no `name` field on an edit, and that is the tool surface rather
/// than an omission**: `kg_task_update` moves a status and edits scheduling,
/// and the graph exposes no rename. Offering a box that silently discarded
/// what was typed in it would be worse than not offering one.
pub struct Form {
    pub editing: Option<String>,
    pub fields: Vec<(&'static str, String)>,
    pub idx: usize,
    /// A refusal from the last submit — an unparseable date, a project the
    /// graph does not have. Shown in the form, which stays open with the
    /// typing intact: bouncing beats saving junk, and beats losing the words.
    pub error: Option<String>,
}

impl Form {
    pub fn capture() -> Self {
        Form {
            editing: None,
            fields: vec![
                ("name", String::new()),
                ("due", String::new()),
                ("project", String::new()),
                ("context", String::new()),
            ],
            idx: 0,
            error: None,
        }
    }

    /// The schedule of an existing task, prefilled with what it currently is
    /// — so an edit that changes one field does not blank the other two.
    pub fn edit(row: &TaskRow) -> Self {
        Form {
            editing: Some(row.id.clone()),
            fields: vec![
                ("due", row.due_at.clone().unwrap_or_default()),
                ("defer", row.defer_until.clone().unwrap_or_default()),
                ("context", row.context.clone().unwrap_or_default()),
            ],
            idx: 0,
            error: None,
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        let len = self.fields.len() as isize;
        self.idx = (((self.idx as isize + delta) % len + len) % len) as usize;
    }

    pub fn push(&mut self, c: char) {
        self.fields[self.idx].1.push(c);
    }

    pub fn backspace(&mut self) {
        self.fields[self.idx].1.pop();
    }

    pub fn value(&self, field: &str) -> &str {
        self.fields
            .iter()
            .find(|(name, _)| *name == field)
            .map(|(_, v)| v.as_str())
            .unwrap_or_default()
    }

    pub fn title(&self) -> String {
        match &self.editing {
            Some(_) => " edit schedule · tab moves · enter saves · esc back ".into(),
            None => " capture a task · tab moves · enter saves · esc back ".into(),
        }
    }
}

pub struct TasksModal {
    pub rows: Vec<TaskRow>,
    pub selected: usize,
    pub detail: bool,
    /// Whether done and dropped are in `rows`. The list is reloaded when it
    /// flips rather than filtered here — the graph decides what "closed"
    /// means and orders the board, and re-implementing either would be a
    /// second board that disagrees with `mecha tasks list`.
    pub show_closed: bool,
    pub form: Option<Form>,
    pub help: bool,
    pub status: Option<String>,
    /// The graph's today, carried so the footer can say what "overdue" was
    /// measured against.
    pub today: String,
}

/// What a key does. Everything here is immediate: see the module note on why
/// nothing on this board confirms.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Set the selected task's status.
    Status(&'static str),
    /// Walk the actionable statuses in order.
    Cycle,
    /// Open the capture form.
    Add,
    /// Open the schedule-edit form.
    Edit,
    /// Show or hide done and dropped.
    Closed,
    /// Re-read the board.
    Refresh,
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

/// Every key this modal answers to, written down once — the `/mail` rule: one
/// table, so a legend cannot advertise a key the map does not answer to.
///
/// **The letters are the graph TUI's letters.** `mecha-graph tui` screen 6 is
/// the same board with the same six status keys, and a person who has learned
/// `d` there must not find it means something else here. Where the two differ
/// it is because this modal has a sibling convention to keep instead —
/// `enter` opens a detail and `q`/`esc` close, as they do in every mecha
/// modal.
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
        short: "↵ open",
        note: "the whole task: full name, id, dates, project",
    },
    Key {
        key: 'a',
        short: "a add",
        note: "capture a task — lands in inbox, like every other capture",
    },
    Key {
        key: 'e',
        short: "e edit",
        note: "due, defer and context of the selected task",
    },
    Key {
        key: 'n',
        short: "n next",
        note: "status → next: committed to, actionable now",
    },
    Key {
        key: 'i',
        short: "i inbox",
        note: "status → inbox: captured, not yet decided on",
    },
    Key {
        key: 's',
        short: "s sched",
        note: "status → scheduled: it has a date and waits for it",
    },
    Key {
        key: 'w',
        short: "w wait",
        note: "status → waiting: blocked on somebody else",
    },
    Key {
        key: 'd',
        short: "d done",
        note: "status → done — reversible: n or i reopens it",
    },
    Key {
        key: 'x',
        short: "x drop",
        note: "status → dropped — reversible too; nothing is ever deleted",
    },
    Key {
        key: ' ',
        short: "spc cycle",
        note: "walk next → inbox → scheduled → waiting",
    },
    Key {
        key: 'z',
        short: "z closed",
        note: "show or hide done and dropped",
    },
    Key {
        key: 'r',
        short: "r reload",
        note: "re-read the board",
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

/// The strip across the top of the list: the verbs, in key order, skipping the
/// ones the title and the shape of a list already teach.
///
/// **The verbs are abbreviated because `?` is the other tier.** Twelve keys
/// spelled out do not fit a line on any terminal, and a strip that runs off
/// the end teaches whichever half the width happened to fit — the failure the
/// legend exists to fix, in a new place. Short enough to read at a glance,
/// with the sentence behind each one a `?` away.
pub fn key_strip() -> String {
    KEYS.iter()
        .filter(|k| !k.short.is_empty())
        .map(|k| k.short)
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The key map.
pub fn action_for(key: char) -> Option<Action> {
    Some(match key {
        'n' => Action::Status("next"),
        'i' => Action::Status("inbox"),
        's' => Action::Status("scheduled"),
        'w' => Action::Status("waiting"),
        'd' => Action::Status("done"),
        'x' => Action::Status("dropped"),
        ' ' => Action::Cycle,
        'a' => Action::Add,
        'e' => Action::Edit,
        'z' => Action::Closed,
        'r' => Action::Refresh,
        'q' => Action::Close,
        _ => return None,
    })
}

impl TasksModal {
    pub fn new(rows: Vec<TaskRow>, today: String) -> Self {
        TasksModal {
            rows,
            selected: 0,
            detail: false,
            show_closed: false,
            form: None,
            help: false,
            status: None,
            today,
        }
    }

    pub fn selected_row(&self) -> Option<&TaskRow> {
        self.rows.get(self.selected)
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let len = self.rows.len() as isize;
        self.selected = (((self.selected as isize + delta) % len + len) % len) as usize;
    }

    /// The status `space` moves the selection to: the next actionable one
    /// after the current. A closed task re-enters the cycle at the front
    /// rather than staying closed — `space` on a done task means "put this
    /// back to work", which is the only reading it can have.
    pub fn next_in_cycle(&self) -> Option<&'static str> {
        let row = self.selected_row()?;
        let at = ACTIVE_STATUSES.iter().position(|s| *s == row.status);
        Some(match at {
            Some(i) => ACTIVE_STATUSES[(i + 1) % ACTIVE_STATUSES.len()],
            None => ACTIVE_STATUSES[0],
        })
    }

    fn counts(&self) -> (usize, usize) {
        let open = self.rows.iter().filter(|r| !r.closed).count();
        let late = self.rows.iter().filter(|r| r.overdue).count();
        (open, late)
    }

    fn title(&self) -> String {
        if let Some(status) = &self.status {
            return format!(" tasks — {status} ");
        }
        let (open, late) = self.counts();
        let late = if late > 0 {
            format!("{late} overdue · ")
        } else {
            String::new()
        };
        let closed = if self.show_closed {
            "closed shown · "
        } else {
            ""
        };
        format!(" tasks — {open} open · {late}{closed}? keys · esc ")
    }

    /// Keep the selection on screen. Same helper the sibling modals use, and
    /// needed for the same reason: the board grows and `j` past the last
    /// visible row would change the status of a task nobody can see.
    fn list_scroll(&self, visible: u16) -> u16 {
        let visible = visible.max(1) as usize;
        (self.selected + 1).saturating_sub(visible) as u16
    }

    pub fn draw(&self, frame: &mut Frame) {
        if self.help {
            self.draw_help(frame);
            return;
        }
        if let Some(form) = &self.form {
            draw_form(frame, form);
            return;
        }
        if self.detail {
            self.draw_detail(frame);
            return;
        }
        self.draw_list(frame);
    }

    fn draw_list(&self, frame: &mut Frame) {
        let strip_text = format!("  {}", key_strip());
        let strip = Line::styled(strip_text.clone(), Style::new().fg(Color::Cyan));
        let body: Vec<Line> = if self.rows.is_empty() {
            vec![Line::styled(
                "  nothing on the board — a captures one",
                Style::new().fg(Color::DarkGray),
            )]
        } else {
            self.rows
                .iter()
                .enumerate()
                .map(|(i, row)| {
                    let selected = i == self.selected;
                    let marker = if selected { "›" } else { " " };
                    // `!` for overdue, in the same column for every row: a
                    // mark that moves is a mark you have to read for.
                    let late = if row.overdue { "!" } else { " " };
                    let text = format!(
                        "{marker} {late} {:<10} {:<11} {:<62} {}",
                        row.status,
                        row.due_at.as_deref().unwrap_or("—"),
                        truncate(&row.name, 62),
                        truncate(&row.tail(), 24),
                    );
                    if selected {
                        Line::styled(text, Style::new().fg(Color::Black).bg(Color::Cyan))
                    } else if row.closed {
                        Line::styled(text, Style::new().fg(Color::DarkGray))
                    } else if row.overdue {
                        Line::styled(text, Style::new().fg(Color::Red))
                    } else {
                        Line::styled(text, Style::new().fg(Color::White))
                    }
                })
                .collect()
        };

        let width = 120u16.min(frame.area().width);
        let strip_lines = strip_height(&strip_text, width.saturating_sub(2));
        let height =
            super::list_height_reserving(body.len() as u16, frame.area().height, strip_lines);
        let area = super::centered(frame.area(), width, height);
        frame.render_widget(Clear, area);
        // The strip renders outside the scrolling paragraph, in the first line
        // of the block — a legend that scrolls away exactly when the board
        // gets big enough to need one is not a legend.
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(Color::Cyan))
            .title(self.title());
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height == 0 {
            return;
        }
        // Wrapped, not truncated: the strip fits one line at the width this
        // modal asks for, and a terminal narrower than that must lose the
        // list's last rows rather than half the legend — a key nobody can see
        // is a key nobody presses.
        let lines = strip_height(&strip_text, inner.width);
        frame.render_widget(
            Paragraph::new(strip).wrap(Wrap { trim: false }),
            Rect {
                height: lines.min(inner.height),
                ..inner
            },
        );
        let list = Rect {
            y: inner.y + lines,
            height: inner.height.saturating_sub(lines),
            ..inner
        };
        frame.render_widget(
            Paragraph::new(body).scroll((self.list_scroll(list.height), 0)),
            list,
        );
    }

    /// The whole task. The list truncates a name to fit a column; this is
    /// where the rest of it lives, along with the id somebody would type at a
    /// terminal.
    fn draw_detail(&self, frame: &mut Frame) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let white = Style::new().fg(Color::White);
        let grey = Style::new().fg(Color::DarkGray);
        let red = Style::new().fg(Color::Red);

        let mut body = vec![
            Line::styled(row.name.clone(), white),
            Line::raw(""),
            Line::styled(
                format!("status    {}", row.status),
                if row.closed { grey } else { white },
            ),
            Line::styled(
                format!("due       {}", row.due_at.as_deref().unwrap_or("—")),
                if row.overdue { red } else { white },
            ),
        ];
        for (label, value) in [
            ("defer", &row.defer_until),
            ("project", &row.project),
            ("context", &row.context),
            ("waiting", &row.waiting_on),
        ] {
            if let Some(v) = value.as_deref().filter(|v| !v.is_empty()) {
                body.push(Line::styled(format!("{label:<9} {v}"), white));
            }
        }
        body.push(Line::raw(""));
        body.push(Line::styled(
            format!("{}  ·  today is {}", row.id, self.today),
            grey,
        ));

        let area = super::centered(
            frame.area(),
            100,
            (body.len() as u16 + 2).min(frame.area().height),
        );
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(body).wrap(Wrap { trim: false }).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(Color::Cyan))
                    .title(" task · e edit · n/i/s/w/d/x status · esc back "),
            ),
            area,
        );
    }

    /// `?` — every key, with what it does.
    fn draw_help(&self, frame: &mut Frame) {
        let body: Vec<Line> = KEYS
            .iter()
            .map(|k| {
                let key = match k.key {
                    '\n' => "enter".to_string(),
                    ' ' => "space".to_string(),
                    c => c.to_string(),
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
                    .title(" task keys · any key closes "),
            ),
            area,
        );
    }
}

impl TaskRow {
    /// Project, context and who it waits on, for the last column — and empty
    /// when there are none, rather than a row of separators describing a task
    /// that has nothing to say.
    pub fn tail(&self) -> String {
        [&self.project, &self.context, &self.waiting_on]
            .iter()
            .filter_map(|v| v.as_deref().filter(|v| !v.is_empty()))
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// The form, one field per line, the cursor on the one being typed.
fn draw_form(frame: &mut Frame, form: &Form) {
    let mut body: Vec<Line> = form
        .fields
        .iter()
        .enumerate()
        .map(|(i, (label, value))| {
            let here = i == form.idx;
            let cursor = if here { "▏" } else { "" };
            Line::from(vec![
                Span::styled(
                    format!("  {label:<9}"),
                    Style::new().fg(if here { Color::Cyan } else { Color::DarkGray }),
                ),
                Span::styled(format!("{value}{cursor}"), Style::new().fg(Color::White)),
            ])
        })
        .collect();
    body.push(Line::raw(""));
    match &form.error {
        // The refusal is the graph's own words. A form that reported "invalid"
        // where the tool said "project 'py-feat' is not a node" would send
        // somebody hunting for a date bug.
        Some(e) => body.push(Line::styled(format!("  {e}"), Style::new().fg(Color::Red))),
        None => body.push(Line::styled(
            "  due takes YYYY-MM-DD, today, tomorrow or +Nd · project must already be in the graph",
            Style::new().fg(Color::DarkGray),
        )),
    }

    let area = super::centered(frame.area(), 96, body.len() as u16 + 2);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Cyan))
                .title(form.title()),
        ),
        area,
    );
}

/// How many lines the legend needs at this width. Ceiling division on the
/// character count: the strip is one flat run of ASCII verbs, so there is no
/// grapheme subtlety to get wrong here.
fn strip_height(strip: &str, width: u16) -> u16 {
    let width = width.max(1) as usize;
    (strip.chars().count().div_ceil(width) as u16).max(1)
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

/// Parse `mecha tasks list --json` — which is `kg_task_list`'s own answer,
/// unchanged — into rows and the graph's today.
///
/// Pure, so the shape of the board can be tested without a graph, a model or
/// a terminal. A missing optional field is an absent one rather than an
/// error: this reads another repository's wire format, and a modal that
/// refuses to draw because a field it never uses was added is the fragile
/// half of that seam.
pub fn rows_from_json(text: &str) -> anyhow::Result<(Vec<TaskRow>, String)> {
    use anyhow::Context;
    let board: serde_json::Value =
        serde_json::from_str(text).context("`mecha tasks list --json` did not answer with JSON")?;
    let today = board["today"].as_str().unwrap_or_default().to_string();
    let rows = board["items"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .map(|t| {
            let text = |key: &str| {
                t[key]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
            };
            TaskRow {
                id: t["id"].as_str().unwrap_or_default().to_string(),
                name: t["name"].as_str().unwrap_or_default().to_string(),
                status: t["status"].as_str().unwrap_or("?").to_string(),
                due_at: text("due_at"),
                defer_until: text("defer_until"),
                context: text("context"),
                project: text("project"),
                waiting_on: text("waiting_on"),
                overdue: t["overdue"].as_bool().unwrap_or(false),
                closed: t["completed_at"].is_string(),
            }
        })
        .collect();
    Ok((rows, today))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: &str = r#"{"v":1,"today":"2026-08-20","truncated":false,"items":[
        {"id":"task-790c1384","name":"Verify suspicious Microsoft invoice email","status":"inbox",
         "due_at":"2026-08-15","defer_until":null,"context":"@email","project":null,
         "waiting_on":null,"completed_at":null,"overdue":true},
        {"id":"task-b34fb2d0","name":"Edit Alexis's Master's thesis","status":"next",
         "due_at":"2026-08-20","defer_until":null,"context":null,"project":"Alexis Cameron",
         "waiting_on":null,"completed_at":null,"overdue":false},
        {"id":"task-dead0000","name":"Something finished","status":"done",
         "due_at":null,"defer_until":null,"context":null,"project":null,
         "waiting_on":null,"completed_at":"2026-08-19","overdue":false}]}"#;

    /// One table, checked both ways — the `/mail` rule. A legend advertising a
    /// key the map does not answer to teaches a keypress that does nothing;
    /// a key the legend omits is a feature nobody finds.
    #[test]
    fn the_legend_and_the_key_map_are_the_same_set() {
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

    /// The six status letters are the graph TUI's six. Somebody who learned
    /// this board in `mecha-graph tui` must not find that `d` means something
    /// else here — two boards over one store with divergent keys is a trap,
    /// and `x` on a task you meant to finish is not one this modal should set.
    #[test]
    fn the_status_letters_match_the_graph_tui() {
        for (key, status) in [
            ('n', "next"),
            ('i', "inbox"),
            ('w', "waiting"),
            ('s', "scheduled"),
            ('d', "done"),
            ('x', "dropped"),
        ] {
            assert_eq!(
                action_for(key),
                Some(Action::Status(status)),
                "`{key}` is {status} in mecha-graph tui screen 6"
            );
        }
    }

    /// The strip is the verbs, not every key: `j`/`k`/`?`/`q` are taught by
    /// the title and by the shape of a list. And it fits the width this modal
    /// asks for — a legend that runs off the end teaches whichever half the
    /// terminal happened to fit, which is the failure it exists to fix.
    #[test]
    fn the_key_strip_names_the_actions_and_fits_a_line() {
        let strip = key_strip();
        for expected in ["a add", "e edit", "d done", "spc cycle", "r reload"] {
            assert!(strip.contains(expected), "{expected} missing from {strip}");
        }
        assert!(!strip.contains(" j "), "{strip}");
        // 120 wide, two borders, two spaces of indent.
        assert!(
            strip.chars().count() <= 116,
            "{} wide: {strip}",
            strip.chars().count()
        );
    }

    /// Narrower than the strip, it wraps rather than truncating: the list can
    /// afford to lose its last row, and a key nobody can see is a key nobody
    /// presses.
    #[test]
    fn a_narrow_terminal_wraps_the_legend_instead_of_cutting_it() {
        assert_eq!(strip_height("abcdef", 100), 1);
        assert_eq!(strip_height("abcdef", 3), 2);
        assert_eq!(strip_height("abcdef", 2), 3);
        assert_eq!(strip_height("", 0), 1, "never zero-height");
    }

    /// The shared helper is called with the legend's rows as `reserved`, and
    /// **the argument order is the thing this pins**: all three parameters are
    /// `u16`, so swapping `terminal_height` and `reserved` compiles, does not
    /// panic, and silently returns a three-row box. Nothing else here would
    /// fail on that — the panic test still passes, because a too-small box is
    /// not a crash. So the numbers are asserted directly.
    #[test]
    fn the_box_reserves_the_legends_rows_and_not_something_else() {
        // 24 rows: ceiling 20. Three tasks and a one-line legend want 4.
        assert_eq!(super::super::list_height_reserving(3, 24, 1), 6);
        // The legend's rows are what a full board loses to, not the list's.
        assert_eq!(super::super::list_height_reserving(100, 24, 1), 22);
        // Argument order, stated as a value: were height and reserved swapped,
        // this would be 3 rather than 6.
        assert_ne!(super::super::list_height_reserving(3, 1, 24), 6);
    }

    /// Every view, at every shape of terminal a person can drag one into.
    ///
    /// The `/doctor` regression, which this box reintroduced with a second
    /// moving bound: `clamp` panics when the ceiling saturates below the
    /// floor, and a panic in `draw` is the whole session. 20x8 is the case a
    /// fixed floor missed — narrow enough that the legend wraps to seven
    /// lines, short enough that four rows are all there is.
    #[test]
    fn no_terminal_size_panics_the_draw() {
        let (rows, today) = rows_from_json(BOARD).unwrap();
        for (w, h) in [
            (130, 24),
            (130, 5),
            (130, 3),
            (130, 1),
            (60, 5),
            (20, 8),
            (20, 3),
            (8, 2),
            (1, 1),
        ] {
            let (rows, today) = (clone_rows(&rows), today.clone());
            let mut modal = TasksModal::new(rows, today);
            for view in 0..4 {
                match view {
                    1 => modal.detail = true,
                    2 => {
                        modal.detail = false;
                        modal.help = true;
                    }
                    3 => {
                        modal.help = false;
                        modal.form = Some(Form::capture());
                    }
                    _ => {}
                }
                let mut terminal =
                    ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
                terminal
                    .draw(|f| modal.draw(f))
                    .unwrap_or_else(|e| panic!("{w}x{h} view {view}: {e}"));
            }
        }
        // Not vacuous: the floor really does exceed the ceiling at 20x8, so
        // an unguarded clamp would have panicked above.
        assert!(strip_height(&format!("  {}", key_strip()), 18) > 8u16.saturating_sub(4));
    }

    fn clone_rows(rows: &[TaskRow]) -> Vec<TaskRow> {
        rows.iter()
            .map(|r| TaskRow {
                id: r.id.clone(),
                name: r.name.clone(),
                status: r.status.clone(),
                due_at: r.due_at.clone(),
                defer_until: r.defer_until.clone(),
                context: r.context.clone(),
                project: r.project.clone(),
                waiting_on: r.waiting_on.clone(),
                overdue: r.overdue,
                closed: r.closed,
            })
            .collect()
    }

    #[test]
    fn the_board_parses_into_rows() {
        let (rows, today) = rows_from_json(BOARD).unwrap();
        assert_eq!(today, "2026-08-20");
        assert_eq!(rows.len(), 3);
        assert!(rows[0].overdue && !rows[0].closed);
        assert_eq!(rows[0].context.as_deref(), Some("@email"));
        assert_eq!(rows[1].tail(), "Alexis Cameron");
        assert!(rows[2].closed, "completed_at is what closes a task");
    }

    /// A null field is an absent one, not the string "null" — which is what
    /// `Value::to_string` would have put on screen, and what a task with no
    /// project would then look like it had.
    #[test]
    fn absent_fields_stay_absent() {
        let (rows, _) = rows_from_json(BOARD).unwrap();
        assert_eq!(rows[0].project, None);
        assert_eq!(rows[1].tail(), "Alexis Cameron", "no empty separators");
        assert_eq!(rows[0].tail(), "@email");
    }

    /// `space` walks the working statuses and never lands on done or dropped:
    /// finishing something is a decision, not the next step in a rotation.
    #[test]
    fn cycling_stays_among_the_actionable_statuses() {
        let (rows, today) = rows_from_json(BOARD).unwrap();
        let mut modal = TasksModal::new(rows, today);
        assert_eq!(
            modal.next_in_cycle(),
            Some("scheduled"),
            "inbox → scheduled"
        );
        modal.selected = 1;
        assert_eq!(modal.next_in_cycle(), Some("inbox"), "next → inbox");
        // The closed one re-enters the cycle at the front rather than being
        // stuck: `space` on a finished task means "put this back to work".
        modal.selected = 2;
        assert_eq!(modal.next_in_cycle(), Some("next"));
    }

    /// An edit form is prefilled, or changing a context would blank a due date
    /// the person never touched.
    #[test]
    fn the_edit_form_starts_from_what_the_task_already_is() {
        let (rows, _) = rows_from_json(BOARD).unwrap();
        let form = Form::edit(&rows[0]);
        assert_eq!(form.editing.as_deref(), Some("task-790c1384"));
        assert_eq!(form.value("due"), "2026-08-15");
        assert_eq!(form.value("context"), "@email");
        assert_eq!(form.value("defer"), "", "it has none, and says so");
    }

    /// The list spends its width on the task, not on the node id — the
    /// `/mail` finding. The id is in the detail, which is where somebody who
    /// wants to type `mecha tasks set <id>` is looking.
    #[test]
    fn the_list_shows_the_task_and_the_detail_shows_the_id() {
        let (rows, today) = rows_from_json(BOARD).unwrap();
        let mut modal = TasksModal::new(rows, today);
        assert!(!render(&modal).contains("task-790c1384"));
        assert!(render(&modal).contains("Verify suspicious Microsoft invoice"));
        assert!(render(&modal).contains("a add"), "the legend draws unasked");
        modal.detail = true;
        assert!(render(&modal).contains("task-790c1384"));
    }

    fn render(modal: &TasksModal) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(130, 24)).unwrap();
        terminal.draw(|f| modal.draw(f)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .chunks(130)
            .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
