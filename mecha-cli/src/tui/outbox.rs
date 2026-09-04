//! The /outbox modal: what is staged to leave the machine, decided without
//! leaving the session.
//!
//! Same two depths as /triggers: the list answers "what is waiting", the
//! detail answers "what exactly would go out" — the same content as
//! `mecha outbox show`, because a review surface that shows less than the
//! command line is a review surface people learn to distrust.
//!
//! Every mutation shells out to `mecha outbox ...` as a child process, for the
//! reasons the triggers modal wrote down: one implementation of releasing, no
//! way for the TUI to do something the command line cannot, and a send that
//! builds a tool surface (MCP servers included) never runs on the event loop.
//!
//! Two decisions of this surface's own:
//!
//! - **Every send confirms.** The triggers modal only confirms delete, because
//!   its other actions are reversible. Nothing about a send is: it is the one
//!   keystroke here after which mail is in a stranger's inbox. A tainted draft
//!   confirms in red with its full arguments on screen, mirroring what the CLI
//!   prints before asking — "approve without reading" is the failure mode the
//!   outbox exists to prevent, and a modal must not make it easier than the
//!   command line does.
//! - **Reject asks for a reason, and the asking is the confirmation.** The
//!   reason is optional, as on the CLI, but the input box means a stray `r`
//!   resolves nothing.

use mecha_core::outbox::{DraftView, OutboxItem, OutboxKind};
use mecha_core::outbox_source::SourceRead;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

pub struct OutboxRow {
    pub id: String,
    /// `pending` | `sent` | `rejected`.
    pub status: String,
    pub kind: OutboxKind,
    pub summary: String,
    /// Drafted with the trifecta armed — third-party text was in context.
    pub tainted: bool,
    pub edited: bool,
    /// What a tainted send's confirmation puts on screen: the draft as it
    /// would be read, for a message; the arguments, for a publish. The CLI
    /// prints the same before its own prompt, and for the same reason — what
    /// is being approved must be what was read, which argues for the readable
    /// form rather than against it.
    pub args_text: String,
    /// The last release attempt's failure, if any — carried so a watch on a
    /// new release can tell a fresh failure from this old one.
    pub error: Option<String>,
    /// The full detail view, prebuilt so drawing is a scroll and nothing else.
    pub detail: Vec<Line<'static>>,
    /// The same item as raw arguments, on `J`. Prebuilt beside the readable
    /// view rather than replacing it: the readable one is what a person
    /// should decide from, and the exact bytes are what a person should be
    /// able to *check* — a review surface that shows less than the command
    /// line is one people learn to distrust, and this is how it keeps showing
    /// as much while leading with the half that answers the question.
    pub raw: Vec<Line<'static>>,
}

impl OutboxRow {
    pub fn pending(&self) -> bool {
        self.status == "pending"
    }

    fn status_style(&self) -> Style {
        match self.status.as_str() {
            "pending" if self.tainted => Style::new().fg(Color::Red),
            "pending" => Style::new().fg(Color::Yellow),
            "sent" => Style::new().fg(Color::Green),
            _ => Style::new().fg(Color::DarkGray),
        }
    }
}

/// A send waiting on a yes. Carries what the confirmation must show: a
/// tainted draft's arguments are printed in full, exactly as the CLI does
/// before its own prompt.
pub struct SendConfirm {
    pub id: String,
    pub summary: String,
    pub tainted: bool,
    pub args_text: String,
    /// The item's error at confirm time, handed to the watch that reports the
    /// release's outcome: only an error that *changed* is the new attempt's.
    pub error_before: Option<String>,
    /// How far down the arguments the reviewer has read.
    ///
    /// A tainted draft puts its arguments on screen in full, and "in full"
    /// was doing no work: a `docs_replace` whose `find` is a whole syllabus
    /// section overflowed the box, and an unscrolled `Paragraph` renders from
    /// the top — so the tail was dropped silently, taking the question and
    /// the `y` prompt with it. The reviewer saw an attacker warning, an
    /// unreadable wall, and no way forward. Approving what you cannot see is
    /// the one failure this surface exists to prevent, so the arguments
    /// scroll and the prompt is pinned to the border where nothing can push
    /// it off.
    pub scroll: u16,
}

/// A rejection's reason being typed. Optional, as on the CLI — but the input
/// box doubles as the confirmation, so a stray `r` resolves nothing.
pub struct ReasonInput {
    pub id: String,
    pub buffer: String,
}

pub struct OutboxModal {
    pub rows: Vec<OutboxRow>,
    pub selected: usize,
    pub detail: bool,
    pub detail_scroll: u16,
    /// Show the arguments as JSON instead of as a message (`J`).
    pub show_raw: bool,
    /// Show resolved items too (`h`).
    ///
    /// **Off by default, and that is the whole fix for a cluttered queue.**
    /// Every send and every rejection stays on file forever — that is the
    /// record, and it is why nothing here deletes — but a decided item is not
    /// work, and a list that keeps it in front of you buries the three drafts
    /// that are. Hidden, never dropped: the count rides in the title so the
    /// record is one keypress away and visibly there.
    pub history: bool,
    /// A send waiting on `y`. Takes the keyboard while it is up.
    pub confirm: Option<SendConfirm>,
    /// A rejection's reason being typed. Takes the keyboard while it is up.
    pub rejecting: Option<ReasonInput>,
    /// The result of the last action, shown in the title bar.
    pub status: Option<String>,
    /// When set, the modal shows only these item ids — the review-now flow
    /// opens on exactly what the finishing run staged, not the whole queue.
    /// The title says so, and `/outbox` (scope `None`) is always the full
    /// view. Reloads preserve it, so acting on one scoped draft does not
    /// suddenly widen the list to the overnight backlog.
    pub scope: Option<Vec<String>>,
}

impl OutboxModal {
    pub fn new(rows: Vec<OutboxRow>) -> Self {
        OutboxModal {
            rows,
            selected: 0,
            detail: false,
            detail_scroll: 0,
            show_raw: false,
            history: false,
            confirm: None,
            rejecting: None,
            status: None,
            scope: None,
        }
    }

    /// The rows actually on screen. `rows` stays the whole record; this is
    /// what `selected` indexes, so hiding history moves the cursor's meaning
    /// with the list rather than leaving it pointing at a row nobody can see.
    pub fn shown(&self) -> Vec<&OutboxRow> {
        self.rows
            .iter()
            .filter(|r| self.history || r.pending())
            .collect()
    }

    /// How many decided items the `h` toggle would reveal.
    pub fn resolved_count(&self) -> usize {
        self.rows.iter().filter(|r| !r.pending()).count()
    }

    pub fn selected_row(&self) -> Option<&OutboxRow> {
        self.shown().get(self.selected).copied()
    }

    /// Flip the history filter, keeping the row under the cursor selected
    /// where it is still on screen. Recomputed by id rather than by index:
    /// the two lists have different lengths, so an index carried across the
    /// toggle names a different draft — which, on a surface whose next
    /// keypress may be `s`, is the whole failure this modal exists to avoid.
    pub fn toggle_history(&mut self) {
        let under_cursor = self.selected_row().map(|r| r.id.clone());
        self.history = !self.history;
        self.selected = under_cursor
            .and_then(|id| self.shown().iter().position(|r| r.id == id))
            .unwrap_or(0);
        self.detail_scroll = 0;
    }

    pub fn move_by(&mut self, delta: isize) {
        let len = self.shown().len() as isize;
        if len == 0 {
            return;
        }
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

    /// The keymap lives in the title, like every modal here: a modal whose
    /// actions are invisible is a modal with one action.
    pub fn title(&self) -> String {
        let scope = if self.scope.is_some() {
            "this run's drafts · "
        } else {
            ""
        };
        match &self.status {
            Some(s) => format!(" outbox · {scope}{s} "),
            None => {
                let pending = self.rows.iter().filter(|r| r.pending()).count();
                let resolved = self.resolved_count();
                let history = match (self.history, resolved) {
                    (_, 0) => String::new(),
                    (true, n) => format!("with {n} resolved · h hides · "),
                    (false, n) => format!("h shows {n} resolved · "),
                };
                format!(
                    " {scope}{pending} pending · {history}enter detail · a approve · e edit · r reject · esc "
                )
            }
        }
    }

    pub fn draw(&self, frame: &mut Frame) {
        if let Some(confirm) = &self.confirm {
            self.draw_confirm(frame, confirm);
        } else if let Some(input) = &self.rejecting {
            draw_reason_input(
                frame,
                &format!(
                    " reject {} — reason (optional) · enter rejects · esc keeps ",
                    input.id
                ),
                &input.buffer,
            );
        } else if self.detail {
            self.draw_detail(frame);
        } else {
            self.draw_list(frame);
        }
    }

    fn draw_list(&self, frame: &mut Frame) {
        let shown = self.shown();
        let body: Vec<Line> = if shown.is_empty() {
            let grey = Style::new().fg(Color::DarkGray);
            match self.resolved_count() {
                // "Nothing pending" and "nothing here" are different answers,
                // and a queue that has been worked through should say which.
                0 => vec![Line::styled(
                    "  outbox empty — calls to [outbox]-routed tools are staged here",
                    grey,
                )],
                n => vec![Line::styled(
                    format!("  nothing pending — h shows the {n} already decided"),
                    grey,
                )],
            }
        } else {
            shown
                .iter()
                .enumerate()
                .map(|(i, row)| {
                    let selected = i == self.selected;
                    let marker = if selected { "›" } else { " " };
                    let text = format!(
                        "{marker} {:<14} {:<9} {:<8} {}{}{}",
                        row.id,
                        row.status,
                        row.kind.as_str(),
                        row.summary,
                        if row.tainted { "  ⚠ tainted" } else { "" },
                        if row.edited { "  (edited)" } else { "" },
                    );
                    if selected {
                        Line::styled(text, Style::new().fg(Color::Black).bg(Color::Cyan))
                    } else {
                        Line::styled(text, row.status_style())
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
        let Some(row) = self.selected_row() else {
            return;
        };
        let (body, what) = if self.show_raw {
            (row.raw.clone(), "J readable")
        } else {
            (row.detail.clone(), "J raw")
        };
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
                        .title(format!(
                            " {} · ↑↓ scroll · a approve · e edit · r reject · {what} · esc back ",
                            row.id
                        )),
                ),
            area,
        );
    }

    /// The approval confirmation. Red when tainted, and then the arguments
    /// are on screen in full — scrollable, because "in full" has to survive
    /// an argument longer than the terminal.
    fn draw_confirm(&self, frame: &mut Frame, confirm: &SendConfirm) {
        let mut body: Vec<Line> = Vec::new();
        if confirm.tainted {
            body.push(Line::styled(
                "⚠ drafted in a conversation holding private data AND third-party",
                Style::new().fg(Color::Red),
            ));
            body.push(Line::styled(
                "content — review these arguments as possibly an attacker's words:",
                Style::new().fg(Color::Red),
            ));
            body.push(Line::raw(""));
            for line in confirm.args_text.lines() {
                body.push(Line::styled(
                    line.to_string(),
                    Style::new().fg(Color::White),
                ));
            }
        }

        // Full height, like the detail view: a confirmation sized to its
        // content is a confirmation that overflows the moment the content is
        // large, which is exactly when reading it matters most.
        let area = super::centered(frame.area(), 90, frame.area().height.saturating_sub(4));
        let inner_width = area.width.saturating_sub(2);
        let inner_height = area.height.saturating_sub(2);

        let paragraph = Paragraph::new(body).wrap(Wrap { trim: false });
        // Measured after wrapping, never from `body.len()`: one long argument
        // is a single `Line` and many rendered rows, so counting logical lines
        // reports a box that fits when it does not.
        let drawn = paragraph.line_count(inner_width) as u16;
        let max_scroll = drawn.saturating_sub(inner_height);
        let scroll = confirm.scroll.min(max_scroll);

        let hint = if max_scroll > 0 {
            format!(
                " y approve · ↑↓ scroll ({} more line(s) below) · any other key keeps it pending ",
                max_scroll.saturating_sub(scroll)
            )
        } else {
            " y approve · any other key keeps it pending ".to_string()
        };

        frame.render_widget(Clear, area);
        frame.render_widget(
            paragraph.scroll((scroll, 0)).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(if confirm.tainted {
                        Color::Red
                    } else {
                        Color::Yellow
                    }))
                    .title(format!(" approve {} — {}? ", confirm.id, confirm.summary))
                    .title_bottom(hint),
            ),
            area,
        );
    }
}

/// A one-line input box for a reason. Shared with the frontdoor modal, which
/// asks the same shape of question for `close` and `needs-info`.
pub fn draw_reason_input(frame: &mut Frame, title: &str, buffer: &str) {
    let body = vec![Line::styled(
        format!("{buffer}▏"),
        Style::new().fg(Color::White),
    )];
    let area = super::centered(frame.area(), 90, 3);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(body).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Yellow))
                .title(title.to_string()),
        ),
        area,
    );
}

/// Build the rows from the store. File reads only — the same rule as
/// `triggers::load`, and the reason it can run from the event loop.
pub fn load() -> anyhow::Result<Vec<OutboxRow>> {
    let store = crate::commands::outbox::open_store()?;
    let mut items = store.items()?;
    // Pending first: they are what the modal is for. Resolved items follow as
    // the record, greyed by their status style.
    items.sort_by_key(|i| i.status != "pending");
    Ok(items.iter().map(row).collect())
}

fn row(item: &OutboxItem) -> OutboxRow {
    OutboxRow {
        id: item.id.clone(),
        status: item.status.clone(),
        kind: item.kind,
        summary: item.summary.clone(),
        tainted: item.taint.trifecta_armed(),
        edited: item.edited(),
        args_text: confirm_text(item),
        error: item.error.clone(),
        // Read once here, never per frame: `detail` is prebuilt so drawing
        // is a scroll and nothing else, and the source read is a file the
        // renderer must not touch sixty times a second.
        detail: detail_lines(item, &crate::commands::outbox::source_reads(item)),
        raw: raw_lines(item),
    }
}

/// The detail view: the same facts as `mecha outbox show`, in the same shape,
/// because two review surfaces that disagree about what a release would do is
/// how the wrong thing gets released.
///
/// **A message leads with the message.** That is the [`OutboxKind::Publish`]
/// rule generalised — a publish's reviewable object is the rendered page, and
/// a message's is the letter, not the JSON around it. The old order put
/// provenance, a jail path and a `{"body_markdown": "Dear Dirk,\n\nThank…"}`
/// wall in front of a reader whose only question is "would I send this?", and
/// asked them to decode escape sequences to answer it. Approving without
/// reading is the failure this whole surface exists to prevent, so making the
/// draft hard to read has a security cost and not a cosmetic one.
///
/// What is *not* dropped: the taint warning and a failed send stay on top in
/// red, every argument still appears (`DraftView` guarantees it), and the
/// exact bytes are one `J` away.
fn detail_lines(item: &OutboxItem, reads: &[SourceRead]) -> Vec<Line<'static>> {
    let mut body: Vec<Line<'static>> = Vec::new();
    let white = Style::new().fg(Color::White);
    let grey = Style::new().fg(Color::DarkGray);
    let header = Style::new().fg(Color::Yellow);
    let red = Style::new().fg(Color::Red);

    // Above everything, both of them: a warning under the fold is a warning
    // that arrives after the decision.
    if item.taint.trifecta_armed() {
        for line in [
            "⚠ drafted in a conversation holding private data AND third-party",
            "content — read this as possibly an attacker's words, not the",
            "assistant's.",
        ] {
            body.push(Line::styled(line, red));
        }
        body.push(Line::raw(""));
    }
    if let Some(error) = &item.error {
        body.push(Line::styled(
            format!("last send attempt failed: {error}"),
            red,
        ));
        body.push(Line::raw(""));
    }

    match item.kind {
        OutboxKind::Message => {
            let view = DraftView::of(&item.args);
            for (name, value) in &view.headers {
                body.push(Line::from(vec![
                    Span::styled(format!("{name:<9} "), grey),
                    Span::styled(value.clone(), white),
                ]));
            }
            if let Some(text) = &view.body {
                body.push(Line::raw(""));
                // The prose, with its own newlines honoured. A blank line in
                // the draft is a paragraph break in the letter, and rendering
                // it as `\n` is what made this unreadable.
                for line in text.lines() {
                    body.push(Line::styled(line.to_string(), white));
                }
            }
            if !view.other.is_empty() {
                body.push(Line::raw(""));
                body.push(Line::styled("other arguments", header));
                for (name, value) in &view.other {
                    body.push(Line::from(vec![
                        Span::styled(format!("{name:<9} "), grey),
                        Span::styled(value.clone(), grey),
                    ]));
                }
            }
            if item.edited() {
                body.push(Line::raw(""));
                body.push(Line::styled("edited since drafting", header));
                for line in mecha_core::outbox::diff_args(&item.args_before, &item.args).lines() {
                    let style = match line.trim_start().chars().next() {
                        Some('+') => Style::new().fg(Color::Green),
                        Some('-') => red,
                        _ => grey,
                    };
                    body.push(Line::styled(line.to_string(), style));
                }
            }
        }
        OutboxKind::Publish => {
            for (label, path) in
                crate::commands::outbox::local_paths(&item.args, item.workspace.as_deref())
            {
                body.push(Line::styled(format!("{label}: {}", path.display()), white));
                if let Some(entry) = crate::commands::outbox::entry_point(&path) {
                    body.push(Line::styled(format!("open  {}", entry.display()), grey));
                }
                if !path.exists() {
                    body.push(Line::styled(
                        "⚠ gone — rendered into a work directory retention may have swept; \
                         re-render before releasing",
                        red,
                    ));
                }
            }
            body.push(Line::raw(""));
            body.push(Line::styled("what a release would publish", header));
            for line in pretty(&item.args).lines() {
                body.push(Line::styled(line.to_string(), white));
            }
        }
    }

    // What the draft answers, below the draft and above the provenance: it is
    // context for the decision rather than the object of it. Cyan-on-grey and
    // headed by where it came from, because these are someone else's words
    // and must never read as a continuation of the letter.
    for read in reads {
        body.push(Line::raw(""));
        body.push(Line::styled(read.heading(), header));
        for line in read.text.lines() {
            body.push(Line::styled(format!("│ {line}"), grey));
        }
    }

    body.push(Line::raw(""));
    for line in provenance(item) {
        body.push(Line::styled(line, grey));
    }
    body
}

/// Where the draft came from, in grey and at the bottom. True, worth keeping,
/// and not the question a reviewer is answering.
fn provenance(item: &OutboxItem) -> Vec<String> {
    let mut out = vec![
        format!("{} · {} · {}", item.kind.as_str(), item.tool, item.status),
        format!("created {}", item.created_at),
    ];
    if let Some(session) = &item.session_id {
        out.push(format!("drafted by session {session}"));
    }
    if let Some(workspace) = &item.workspace {
        out.push(format!("jailed to {}", workspace.display()));
    }
    if let Some(resolved) = &item.resolved_at {
        out.push(format!(
            "resolved {resolved}{}",
            item.reason
                .as_deref()
                .map(|r| format!(" — {r}"))
                .unwrap_or_default()
        ));
    }
    out
}

/// The draft as a confirmation should show it — the same reshaping as the
/// detail, flattened to text because that is what the confirmation box takes.
fn confirm_text(item: &OutboxItem) -> String {
    if item.kind != OutboxKind::Message {
        return pretty(&item.args);
    }
    let view = DraftView::of(&item.args);
    let mut out: Vec<String> = view
        .headers
        .iter()
        .map(|(k, v)| format!("{k:<9} {v}"))
        .collect();
    if let Some(body) = &view.body {
        out.push(String::new());
        out.extend(body.lines().map(String::from));
    }
    if !view.other.is_empty() {
        out.push(String::new());
        out.extend(view.other.iter().map(|(k, v)| format!("{k:<9} {v}")));
    }
    out.join("\n")
}

/// The exact arguments, on `J`. What the detail used to lead with, kept as
/// what it always should have been: the check, not the read.
fn raw_lines(item: &OutboxItem) -> Vec<Line<'static>> {
    let mut body = vec![Line::styled(
        "arguments a release would execute",
        Style::new().fg(Color::Yellow),
    )];
    for line in pretty(&item.args).lines() {
        body.push(Line::styled(
            line.to_string(),
            Style::new().fg(Color::White),
        ));
    }
    body
}

fn pretty(v: &serde_json::Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn text(lines: &[Line]) -> String {
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

    fn item(id: &str, status: &str, kind: OutboxKind) -> OutboxItem {
        OutboxItem {
            author: Default::default(),
            filled_defaults: Vec::new(),
            call_id: None,
            id: id.into(),
            status: status.into(),
            tool: "mail__mail_send".into(),
            kind,
            args_before: json!({"to": "a@example.com", "body": "hi"}),
            args: json!({"to": "a@example.com", "body": "hi"}),
            summary: "mail to a@example.com".into(),
            session_id: None,
            workspace: None,
            taint: Default::default(),
            created_at: "2026-08-08T07:00:00Z".into(),
            resolved_at: None,
            reason: None,
            error: None,
        }
    }

    fn rows() -> Vec<OutboxRow> {
        vec![
            row(&item("aaa1", "pending", OutboxKind::Message)),
            row(&item("bbb1", "sent", OutboxKind::Message)),
        ]
    }

    #[test]
    fn the_title_counts_pending_and_names_the_keys() {
        let modal = OutboxModal::new(rows());
        let title = modal.title();
        assert!(title.contains("1 pending"), "{title}");
        // The record is hidden, not gone, and the title is where that is
        // said — a filter nobody can see is a queue that looks shorter than
        // it is.
        assert!(title.contains("h shows 1 resolved"), "{title}");
        for key in ["enter", "a approve", "e edit", "r reject", "esc"] {
            assert!(title.contains(key), "{key} missing from {title}");
        }

        // A status message replaces the keymap: it is the answer to what was
        // just pressed.
        let done = OutboxModal {
            status: Some("rejected `aaa1`".into()),
            ..modal
        };
        assert!(done.title().contains("rejected `aaa1`"));
    }

    /// The review-now flow opens on one run's drafts, and the title has to
    /// say so — a narrowed list that looks like the whole queue reads as
    /// "everything else got handled".
    #[test]
    fn a_scoped_modal_says_whose_drafts_it_is_showing() {
        let scoped = OutboxModal {
            scope: Some(vec!["aaa1".into()]),
            ..OutboxModal::new(rows())
        };
        assert!(
            scoped.title().contains("this run's drafts"),
            "{}",
            scoped.title()
        );

        // The scope marker survives a status message: "sent" over a scoped
        // list must not silently re-widen what the list claims to be.
        let with_status = OutboxModal {
            status: Some("sent `aaa1`".into()),
            ..scoped
        };
        assert!(
            with_status.title().contains("this run's drafts"),
            "{}",
            with_status.title()
        );

        assert!(
            !OutboxModal::new(rows()).title().contains("this run"),
            "the full queue does not claim a scope"
        );
    }

    #[test]
    fn the_selection_wraps_and_an_empty_list_does_not_panic() {
        let mut modal = OutboxModal {
            history: true,
            ..OutboxModal::new(rows())
        };
        modal.move_by(-1);
        assert_eq!(modal.selected, 1);
        modal.move_by(1);
        assert_eq!(modal.selected, 0);

        let mut empty = OutboxModal::new(Vec::new());
        empty.move_by(1);
        assert_eq!(empty.selected, 0);
        assert!(empty.selected_row().is_none());
    }

    /// A decided draft is not work. It stays on file forever — that is the
    /// record — but the queue is what is still to be answered, and a list
    /// where three pending drafts sit under thirty resolved ones is a list
    /// people stop reading.
    #[test]
    fn resolved_items_are_hidden_until_asked_for() {
        let mut modal = OutboxModal::new(rows());
        assert_eq!(modal.shown().len(), 1);
        assert_eq!(modal.selected_row().map(|r| r.id.as_str()), Some("aaa1"));
        modal.toggle_history();
        assert_eq!(modal.shown().len(), 2);
        // The row under the cursor stays under the cursor: the two lists have
        // different lengths, and an index carried across the toggle would
        // name a different draft to the next `s`.
        assert_eq!(modal.selected_row().map(|r| r.id.as_str()), Some("aaa1"));
        modal.toggle_history();
        assert_eq!(modal.selected_row().map(|r| r.id.as_str()), Some("aaa1"));
    }

    /// Selecting a resolved row and hiding history must not leave the cursor
    /// pointing past the end of the list — the next keypress may be `s`.
    #[test]
    fn hiding_history_from_a_resolved_row_lands_somewhere_real() {
        let mut modal = OutboxModal {
            history: true,
            selected: 1,
            ..OutboxModal::new(rows())
        };
        assert_eq!(modal.selected_row().map(|r| r.id.as_str()), Some("bbb1"));
        modal.toggle_history();
        assert_eq!(modal.selected_row().map(|r| r.id.as_str()), Some("aaa1"));
    }

    #[test]
    fn moving_resets_the_detail_scroll() {
        let mut modal = OutboxModal::new(rows());
        modal.scroll_detail(15);
        modal.move_by(1);
        assert_eq!(modal.detail_scroll, 0);
    }

    /// A message's detail leads with the message. The provenance is true and
    /// belongs at the bottom; the taint warning belongs above everything.
    #[test]
    fn the_detail_leads_with_the_letter_and_ends_with_the_provenance() {
        let item = item("aaa1", "pending", OutboxKind::Message);
        let body = text(&detail_lines(&item, &[]));
        let letter = body.find("hi").expect("the body is shown");
        let created = body.find("created").expect("the provenance is kept");
        assert!(letter < created, "the draft comes first:\n{body}");
        assert!(
            !body.contains("body_markdown"),
            "no JSON in the read:\n{body}"
        );
        // Every byte is still one keypress away.
        let raw = text(&raw_lines(&item));
        assert!(raw.contains("arguments a release would execute"), "{raw}");
        assert!(raw.contains("\"body\""), "{raw}");
    }

    /// The prose is shown with its own line breaks. Rendering a paragraph
    /// break as `\n` is what made a draft something to decode rather than
    /// read.
    #[test]
    fn the_body_keeps_its_newlines() {
        let mut item = item("aaa1", "pending", OutboxKind::Message);
        item.args = json!({"to": "a@example.com", "body_markdown": "Dear A,\n\nHello.\n\nLuke"});
        let body = text(&detail_lines(&item, &[]));
        assert!(body.contains("\nDear A,\n\nHello.\n\nLuke\n"), "{body}");
    }

    #[test]
    fn the_detail_shows_the_release_arguments_and_the_taint_warning() {
        let clean = detail_lines(&item("aaa1", "pending", OutboxKind::Message), &[]);
        let body = text(&clean);
        assert!(body.contains("a@example.com"), "{body}");
        assert!(
            !body.contains("attacker"),
            "untainted drafts warn of nothing"
        );

        let mut tainted = item("aaa1", "pending", OutboxKind::Message);
        tainted.taint = mecha_core::agent::Taint {
            private: true,
            untrusted: true,
        };
        let body = text(&detail_lines(&tainted, &[]));
        assert!(body.contains("attacker"), "{body}");
    }

    #[test]
    fn the_source_read_sits_below_the_letter_and_never_reads_as_part_of_it() {
        let read = SourceRead {
            tool: "mail__mail_get_thread".into(),
            keys: vec!["thread_id".into()],
            join: mecha_core::outbox_source::Join::Asked,
            text: "Dear Dr. Chang,\n\nI am an incoming freshman.".into(),
        };
        let body = text(&detail_lines(
            &item("aaa1", "pending", OutboxKind::Message),
            std::slice::from_ref(&read),
        ));
        // Split on the shared heading rather than on a literal, so a reworded
        // lead cannot make this assert against a string no surface prints.
        let (draft, source) = body
            .split_once(read.heading().split(" — ").next().unwrap())
            .expect("the source read is headed, not appended silently");
        assert!(draft.contains("\nhi\n"), "the draft comes first: {body}");
        assert!(
            source.contains("mail__mail_get_thread") && source.contains("third-party"),
            "the heading names the tool and says whose words these are: {body}"
        );
        // Gutter-marked, so a reader scrolling past cannot mistake a
        // stranger's paragraph for the assistant's.
        assert!(source.contains("│ Dear Dr. Chang,"), "{body}");
        // And below the letter rather than above it: a reviewer's question is
        // "would I send this?", which is answered by the draft.
        assert!(body.find("\nhi\n") < body.find("Dear Dr. Chang,"), "{body}");
    }

    #[test]
    fn an_edited_item_shows_the_diff_the_learning_capture_will_mine() {
        let mut edited = item("aaa1", "pending", OutboxKind::Message);
        edited.args = json!({"to": "a@example.com", "body": "hello"});
        let body = text(&detail_lines(&edited, &[]));
        assert!(body.contains("edited since drafting"), "{body}");
        assert!(body.contains("hello"), "{body}");
    }

    /// A publish's reviewable object is the rendered page. The detail leads
    /// with where it is, and says loudly when retention already swept it.
    #[test]
    fn a_publish_detail_leads_with_the_page_and_warns_when_it_is_gone() {
        let mut publish = item("bbb1", "pending", OutboxKind::Publish);
        publish.args = json!({"bundle": "/nonexistent/bundle-dir", "visibility": "public"});
        let body = text(&detail_lines(&publish, &[]));
        assert!(
            body.contains("rendered bundle: /nonexistent/bundle-dir"),
            "{body}"
        );
        assert!(body.contains("⚠ gone"), "{body}");
        assert!(body.contains("what a release would publish"), "{body}");
    }

    #[test]
    fn pending_sorts_first_so_the_queue_opens_on_what_needs_deciding() {
        // `load` sorts the store's items; the rule is stable-sort on
        // "not pending", which keeps store order within each group.
        let mut items = [
            item("sent1", "sent", OutboxKind::Message),
            item("pend1", "pending", OutboxKind::Message),
            item("pend2", "pending", OutboxKind::Message),
        ];
        items.sort_by_key(|i| i.status != "pending");
        assert_eq!(
            items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["pend1", "pend2", "sent1"]
        );
    }
    /// Fails on the old inline `clamp(1, height.saturating_sub(4))`, which
    /// panicked with `min > max` the moment the terminal was four rows or
    /// fewer — the /doctor bug (F9), which every modal had a copy of because
    /// each new one is written by opening whichever sibling is nearest.
    #[test]
    fn a_tiny_terminal_shrinks_the_list_rather_than_panicking() {
        let modal = OutboxModal::new(rows());
        for height in 0..=6u16 {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, height.max(1)))
                    .unwrap();
            // The draw itself is the assertion: the old code panicked here.
            terminal.draw(|f| modal.draw(f)).unwrap();
        }
    }

    fn rendered(modal: &OutboxModal, w: u16, h: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| modal.draw(f)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    fn confirming(args: &str) -> OutboxModal {
        OutboxModal {
            confirm: Some(SendConfirm {
                id: "aaa1".into(),
                summary: "docs__docs_replace".into(),
                tainted: true,
                args_text: args.into(),
                error_before: None,
                scroll: 0,
            }),
            ..OutboxModal::new(rows())
        }
    }

    /// The recorded bug: a `docs_replace` whose `find` was a whole syllabus
    /// section overflowed the confirmation, and an unscrolled `Paragraph`
    /// renders from the top — so the tail was dropped, taking the question
    /// and the `y` prompt with it. The reviewer saw an attacker warning, a
    /// wall of text, and no way forward.
    ///
    /// Fails on the old body-inlined prompt, which is clipped away here.
    #[test]
    fn the_prompt_survives_arguments_longer_than_the_terminal() {
        let long = (0..200)
            .map(|i| format!("schedule line {i} with enough width to wrap on a narrow box"))
            .collect::<Vec<_>>()
            .join("\n");
        let screen = rendered(&confirming(&long), 100, 24);
        assert!(
            screen.contains("y approve"),
            "the approve prompt was pushed off screen"
        );
        assert!(
            screen.contains("more line(s) below"),
            "nothing told the reviewer there was more to read"
        );
    }

    /// Scrolling has to actually move the arguments, or "review these in
    /// full" is a claim the box cannot honour.
    #[test]
    fn scrolling_moves_through_the_arguments() {
        let long = (0..200)
            .map(|i| format!("schedule line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let top = rendered(&confirming(&long), 100, 24);
        assert!(top.contains("schedule line 0"), "top should show the start");

        let mut scrolled = confirming(&long);
        scrolled.confirm.as_mut().unwrap().scroll = 60;
        let lower = rendered(&scrolled, 100, 24);
        assert!(
            !lower.contains("schedule line 0 "),
            "scrolling did not move the view"
        );
        assert!(
            lower.contains("y approve"),
            "the prompt must stay pinned while scrolling"
        );
    }

    /// A short draft needs no scroll furniture — the hint would be a lie.
    #[test]
    fn a_short_draft_gets_no_scroll_hint() {
        let screen = rendered(&confirming("find  Spring 2024"), 100, 24);
        assert!(screen.contains("y approve"));
        assert!(!screen.contains("more line(s) below"));
    }
}
