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

use mecha_core::outbox::{OutboxItem, OutboxKind};
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
    /// The exact arguments a release would execute, pretty-printed. Shown in
    /// the detail and in a tainted send's confirmation.
    pub args_text: String,
    /// The last release attempt's failure, if any — carried so a watch on a
    /// new release can tell a fresh failure from this old one.
    pub error: Option<String>,
    /// The full detail view, prebuilt so drawing is a scroll and nothing else.
    pub detail: Vec<Line<'static>>,
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
            confirm: None,
            rejecting: None,
            status: None,
            scope: None,
        }
    }

    pub fn selected_row(&self) -> Option<&OutboxRow> {
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
                format!(
                    " {scope}{pending} pending of {} · enter detail · s send · e edit · r reject · esc ",
                    self.rows.len()
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
        let body: Vec<Line> = if self.rows.is_empty() {
            vec![Line::styled(
                "  outbox empty — calls to [outbox]-routed tools are staged here",
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
                            " {} · ↑↓ scroll · s send · e edit · r reject · esc back ",
                            row.id
                        )),
                ),
            area,
        );
    }

    /// The send confirmation. Red when tainted, and then the arguments are on
    /// screen in full: what is being approved must be what was read.
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
            body.push(Line::raw(""));
        }
        body.push(Line::styled(
            format!("send {} — {}?", confirm.id, confirm.summary),
            Style::new().fg(Color::White),
        ));
        body.push(Line::raw(""));
        body.push(Line::styled(
            "y sends it for real · anything else keeps it pending",
            Style::new().fg(Color::DarkGray),
        ));

        let height = (body.len() as u16 + 2).min(frame.area().height.saturating_sub(4));
        let area = super::centered(frame.area(), 90, height);
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(body).wrap(Wrap { trim: false }).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(if confirm.tainted {
                        Color::Red
                    } else {
                        Color::Yellow
                    }))
                    .title(" confirm send "),
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
        args_text: pretty(&item.args),
        error: item.error.clone(),
        detail: detail_lines(item),
    }
}

/// The detail view: the same facts as `mecha outbox show`, in the same order,
/// because two review surfaces that disagree about what a release would do is
/// how the wrong thing gets released.
fn detail_lines(item: &OutboxItem) -> Vec<Line<'static>> {
    let mut body: Vec<Line<'static>> = Vec::new();
    let white = Style::new().fg(Color::White);
    let grey = Style::new().fg(Color::DarkGray);
    let header = Style::new().fg(Color::Yellow);

    body.push(Line::styled(
        format!("{} · {} · {}", item.kind.as_str(), item.tool, item.status),
        white,
    ));
    body.push(Line::styled(format!("created {}", item.created_at), grey));
    if let Some(session) = &item.session_id {
        body.push(Line::styled(format!("drafted by session {session}"), grey));
    }
    if let Some(workspace) = &item.workspace {
        body.push(Line::styled(
            format!("jailed to {}", workspace.display()),
            grey,
        ));
    }
    if item.taint.trifecta_armed() {
        body.push(Line::raw(""));
        for line in [
            "⚠ drafted in a conversation holding private data AND third-party",
            "content — review these arguments as possibly an attacker's words,",
            "not the assistant's.",
        ] {
            body.push(Line::styled(line, Style::new().fg(Color::Red)));
        }
    }
    if let Some(resolved) = &item.resolved_at {
        body.push(Line::styled(
            format!(
                "resolved {resolved}{}",
                item.reason
                    .as_deref()
                    .map(|r| format!(" — {r}"))
                    .unwrap_or_default()
            ),
            grey,
        ));
    }
    if let Some(error) = &item.error {
        body.push(Line::styled(
            format!("last send attempt failed: {error}"),
            Style::new().fg(Color::Red),
        ));
    }

    body.push(Line::raw(""));
    match item.kind {
        OutboxKind::Message => {
            body.push(Line::styled("arguments a release would execute", header));
            for line in pretty(&item.args).lines() {
                body.push(Line::styled(line.to_string(), white));
            }
            if item.edited() {
                body.push(Line::raw(""));
                body.push(Line::styled("edited since drafting", header));
                for line in mecha_core::outbox::diff_args(&item.args_before, &item.args).lines() {
                    let style = match line.trim_start().chars().next() {
                        Some('+') => Style::new().fg(Color::Green),
                        Some('-') => Style::new().fg(Color::Red),
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
                        Style::new().fg(Color::Red),
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
    fn the_title_counts_pending_against_the_whole_and_names_the_keys() {
        let modal = OutboxModal::new(rows());
        let title = modal.title();
        assert!(title.contains("1 pending of 2"), "{title}");
        for key in ["enter", "s send", "e edit", "r reject", "esc"] {
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
        let mut modal = OutboxModal::new(rows());
        modal.move_by(-1);
        assert_eq!(modal.selected, 1);
        modal.move_by(1);
        assert_eq!(modal.selected, 0);

        let mut empty = OutboxModal::new(Vec::new());
        empty.move_by(1);
        assert_eq!(empty.selected, 0);
        assert!(empty.selected_row().is_none());
    }

    #[test]
    fn moving_resets_the_detail_scroll() {
        let mut modal = OutboxModal::new(rows());
        modal.scroll_detail(15);
        modal.move_by(1);
        assert_eq!(modal.detail_scroll, 0);
    }

    /// The detail is `show` in a modal: the arguments a release would execute,
    /// the provenance, and the taint warning when it applies.
    #[test]
    fn the_detail_shows_the_release_arguments_and_the_taint_warning() {
        let clean = detail_lines(&item("aaa1", "pending", OutboxKind::Message));
        let body = text(&clean);
        assert!(body.contains("arguments a release would execute"), "{body}");
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
        let body = text(&detail_lines(&tainted));
        assert!(body.contains("attacker"), "{body}");
    }

    #[test]
    fn an_edited_item_shows_the_diff_the_learning_capture_will_mine() {
        let mut edited = item("aaa1", "pending", OutboxKind::Message);
        edited.args = json!({"to": "a@example.com", "body": "hello"});
        let body = text(&detail_lines(&edited));
        assert!(body.contains("edited since drafting"), "{body}");
        assert!(body.contains("hello"), "{body}");
    }

    /// A publish's reviewable object is the rendered page. The detail leads
    /// with where it is, and says loudly when retention already swept it.
    #[test]
    fn a_publish_detail_leads_with_the_page_and_warns_when_it_is_gone() {
        let mut publish = item("bbb1", "pending", OutboxKind::Publish);
        publish.args = json!({"bundle": "/nonexistent/bundle-dir", "visibility": "public"});
        let body = text(&detail_lines(&publish));
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
}
