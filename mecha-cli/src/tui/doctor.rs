//! The /doctor modal: every store's distress, read without leaving the session.
//!
//! The examination comes from running `mecha doctor --json` as a child process
//! — the self-CLI rule every modal here follows: one implementation of the
//! examination, and no way for the TUI to see something the command line
//! cannot. Exit 1 is the diagnosis (findings exist), not a failure of doctor
//! itself, so the loader reads the JSON regardless of which of the two
//! documented codes came back.
//!
//! Acting on a finding dispatches on the remedy's shape, and the decision is
//! [`dispatch`], a pure function, so the three arms are testable without a
//! terminal:
//!
//! - **A remedy whose surface already lives in the TUI deep-links to that
//!   modal** (`mecha outbox review` → /outbox, `mecha frontdoor …` →
//!   /frontdoor) instead of spawning a nested CLI inside the TUI's own
//!   terminal. This wins over everything, including `needs_terminal`: the CLI
//!   marks `outbox review` as needing the terminal because *it* shells out to
//!   `$EDITOR`, but the TUI *is* that review surface.
//! - **A `needs_terminal` remedy suspends the TUI** and hands the real
//!   terminal over — an OAuth flow needs a keyboard and a screen, and
//!   capturing either is the `self_cli_interactive` bug.
//! - **Anything else confirms with y/N, spawns detached, and is watched**:
//!   the outcome reported is a fresh examination, never the child's exit code
//!   alone, because a restarted unit can refail on its next tick and the exit
//!   says nothing about that.

use mecha_core::doctor::{Finding, Remedy, Severity};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

#[derive(Debug)]
pub struct FindingRow {
    pub component: String,
    pub severity: Severity,
    pub summary: String,
    /// The way out, if the finding carries one. Cloned into the dispatch when
    /// `a` is pressed.
    pub remedy: Option<Remedy>,
    /// The full detail view, prebuilt like the outbox rows.
    pub detail: Vec<Line<'static>>,
}

/// What acting on a finding does, decided from the remedy alone so the
/// decision is a unit test rather than a pty session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemedyDispatch {
    /// The remedy's surface is already a TUI modal — switch to it directly.
    /// Deep-linking beats spawning a nested CLI inside the TUI.
    DeepLink(super::command::Command),
    /// Needs the real terminal (an OAuth flow, an `$EDITOR`): suspend the TUI
    /// and hand the terminal over, then re-examine on return.
    Interactive,
    /// Anything else: confirm with y/N, spawn detached, and report the
    /// outcome from a fresh examination.
    Spawn,
}

pub fn dispatch(remedy: &Remedy) -> RemedyDispatch {
    // Deep-link first, deliberately ahead of `needs_terminal`: the CLI marks
    // `mecha outbox review` as needing the terminal because on the command
    // line it opens `$EDITOR`, but here the modal *is* the review surface.
    if let Some(cmd) = deep_link(&remedy.argv) {
        return RemedyDispatch::DeepLink(cmd);
    }
    if remedy.needs_terminal {
        RemedyDispatch::Interactive
    } else {
        RemedyDispatch::Spawn
    }
}

/// The mecha subcommands that have a modal of their own here. Anything else —
/// including other `mecha` verbs — is not a deep link, because a modal that
/// does not exist cannot be switched to.
fn deep_link(argv: &[String]) -> Option<super::command::Command> {
    use super::command::Command;
    let mut parts = argv.iter().map(String::as_str);
    if parts.next()? != "mecha" {
        return None;
    }
    match parts.next()? {
        "outbox" => Some(Command::Outbox),
        "frontdoor" => Some(Command::Frontdoor),
        _ => None,
    }
}

/// A spawnable remedy waiting on a yes. Same rule as an outbox send: the one
/// keystroke that changes the machine is the one that asks first.
pub struct RemedyConfirm {
    pub description: String,
    pub argv: Vec<String>,
}

pub struct DoctorModal {
    pub rows: Vec<FindingRow>,
    pub selected: usize,
    pub detail: bool,
    pub detail_scroll: u16,
    /// A remedy waiting on `y`. Takes the keyboard while it is up.
    pub confirm: Option<RemedyConfirm>,
    /// The result of the last action, shown in the title bar.
    pub status: Option<String>,
    /// An examination is in flight (F7): the child runs detached and a watch
    /// installs the rows when it answers, so the modal opens at once and the
    /// event loop — steering included — stays live while `systemctl` takes
    /// its time. The rows on screen meanwhile are the previous examination's,
    /// which are real findings, just not fresh ones.
    pub examining: bool,
}

impl DoctorModal {
    pub fn new(rows: Vec<FindingRow>) -> Self {
        DoctorModal {
            rows,
            selected: 0,
            detail: false,
            detail_scroll: 0,
            confirm: None,
            status: None,
            examining: false,
        }
    }

    /// The modal as `/doctor` first opens it: no rows yet, examination
    /// running. What the person sees immediately, instead of a frozen frame.
    pub fn examining() -> Self {
        DoctorModal {
            examining: true,
            ..DoctorModal::new(Vec::new())
        }
    }

    pub fn selected_row(&self) -> Option<&FindingRow> {
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
        if self.examining {
            return " doctor · examining — reading every store… · esc ".into();
        }
        if let Some(s) = &self.status {
            return format!(" doctor · {s} ");
        }
        if self.rows.is_empty() {
            return " doctor · nothing wrong that this doctor can see · r re-examine · esc ".into();
        }
        let broken = self
            .rows
            .iter()
            .filter(|r| r.severity == Severity::Broken)
            .count();
        format!(
            " {} finding(s) · {} broken · enter detail · a act · r re-examine · esc ",
            self.rows.len(),
            broken
        )
    }

    pub fn draw(&self, frame: &mut Frame) {
        if let Some(confirm) = &self.confirm {
            draw_confirm(frame, confirm);
        } else if self.detail {
            self.draw_detail(frame);
        } else {
            self.draw_list(frame);
        }
    }

    fn draw_list(&self, frame: &mut Frame) {
        let body: Vec<Line> = if self.rows.is_empty() {
            vec![Line::styled(
                if self.examining {
                    "  examining — reading every store…"
                } else {
                    "  nothing wrong that this doctor can see"
                },
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
                        "{marker} {:<10} [{:<9}] {}{}",
                        row.component,
                        row.severity.as_str(),
                        row.summary,
                        if row.remedy.is_some() {
                            "  · remedy"
                        } else {
                            ""
                        },
                    );
                    if selected {
                        Line::styled(text, Style::new().fg(Color::Black).bg(Color::Cyan))
                    } else if row.severity == Severity::Broken {
                        Line::styled(text, Style::new().fg(Color::Red))
                    } else {
                        Line::styled(text, Style::new().fg(Color::Yellow))
                    }
                })
                .collect()
        };

        let height = list_height(body.len() as u16, frame.area().height);
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
                            " {} · ↑↓ scroll · a act · esc back ",
                            row.component
                        )),
                ),
            area,
        );
    }
}

/// The list box's height for a terminal `terminal_height` rows tall (F9).
///
/// The old inline `clamp(1, height.saturating_sub(4))` violated `min <= max`
/// — a panic — the moment the terminal shrank to four rows or fewer, because
/// the subtraction saturates to zero. The upper bound is floored at one row
/// instead, so a tiny terminal degrades to a one-row box rather than a crash.
fn list_height(rows: u16, terminal_height: u16) -> u16 {
    let max = terminal_height.saturating_sub(4).max(1);
    rows.clamp(1, max) + 2
}

/// The y/N gate for a spawnable remedy, with the command line on screen: what
/// is being approved must be what was read.
fn draw_confirm(frame: &mut Frame, confirm: &RemedyConfirm) {
    let mut body: Vec<Line> = Vec::new();
    for line in confirm.description.lines() {
        body.push(Line::styled(
            line.to_string(),
            Style::new().fg(Color::White),
        ));
    }
    body.push(Line::raw(""));
    body.push(Line::styled(
        format!("run `{}`?", confirm.argv.join(" ")),
        Style::new().fg(Color::White),
    ));
    body.push(Line::raw(""));
    body.push(Line::styled(
        "y runs it · anything else leaves things as they are",
        Style::new().fg(Color::DarkGray),
    ));

    let height = (body.len() as u16 + 2).min(frame.area().height.saturating_sub(4));
    let area = super::centered(frame.area(), 90, height);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Yellow))
                .title(" confirm remedy "),
        ),
        area,
    );
}

/// Start `mecha doctor --json` detached (F7). The examination reads every
/// store and shells out to `systemctl --user`, which on a sick D-Bus can take
/// tens of seconds — run synchronously it froze rendering and steering, the
/// two things the TUI exists to keep live. The child is polled by a
/// `Watch::Examine` (the same exception `Watch::Remedy` documents: the child
/// is the cue, the parsed JSON is the outcome), and the modal shows
/// "examining…" meanwhile.
pub fn spawn_examination() -> anyhow::Result<std::process::Child> {
    use anyhow::Context;
    let exe = std::env::current_exe().context("cannot find my own binary")?;
    std::process::Command::new(exe)
        .args(["doctor", "--json"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("running mecha doctor")
}

/// Collect an exited examination's answer. Only called once the watch saw the
/// child exit, so this never blocks the loop on a running child.
pub fn finish_examination(child: std::process::Child) -> anyhow::Result<Vec<FindingRow>> {
    use anyhow::Context;
    let out = child
        .wait_with_output()
        .context("collecting the examination")?;
    parse_examination(&out.stdout, &out.stderr)
}

/// Exit 1 means findings, not failure — the JSON on stdout is the answer for
/// both documented codes, and only unparseable output is an error worth
/// surfacing (with stderr's first line as the why).
pub fn parse_examination(stdout: &[u8], stderr: &[u8]) -> anyhow::Result<Vec<FindingRow>> {
    let stdout = String::from_utf8_lossy(stdout);
    let findings: Vec<Finding> = serde_json::from_str(stdout.trim()).map_err(|_| {
        let err = String::from_utf8_lossy(stderr);
        anyhow::anyhow!(
            "doctor did not answer with findings: {}",
            err.trim().lines().next().unwrap_or("no output")
        )
    })?;
    Ok(rows(findings))
}

/// How long an examination child may run before the watch kills it and says
/// so. Generous next to the ~25s a wedged `systemctl` can take, small next to
/// the run timeouts.
pub const EXAMINE_CAP: std::time::Duration = std::time::Duration::from_secs(120);

/// What the remedy watch does at one poll, given how long the child has run
/// and how many still-running notices were already posted (F5).
///
/// The old arm posted one notice at five minutes and *dropped the watch* —
/// the `Child` leaked as a zombie and a twelve-minute trigger run's promised
/// outcome never landed. The watch is re-pushed instead: a notice every five
/// minutes so the person knows it is still alive, and a hard cap on the
/// run-timeout scale (a trigger run gets twenty minutes) at which the child
/// is killed, reaped, and reported honestly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemedyPoll {
    /// Keep the watch; nothing to say yet.
    Wait,
    /// Post a still-running notice, then keep the watch.
    Notice,
    /// Past the hard cap: kill, reap, and report that it never finished.
    Kill,
}

pub const REMEDY_NOTICE_EVERY: std::time::Duration = std::time::Duration::from_secs(300);
pub const REMEDY_HARD_CAP: std::time::Duration = std::time::Duration::from_secs(25 * 60);

pub fn remedy_poll(elapsed: std::time::Duration, notices_posted: u32) -> RemedyPoll {
    if elapsed >= REMEDY_HARD_CAP {
        RemedyPoll::Kill
    } else if elapsed >= REMEDY_NOTICE_EVERY.saturating_mul(notices_posted + 1) {
        RemedyPoll::Notice
    } else {
        RemedyPoll::Wait
    }
}

/// Rows in display order: grouped by component, broken before attention —
/// the same shape `mecha doctor` prints, via the same function.
pub fn rows(findings: Vec<Finding>) -> Vec<FindingRow> {
    crate::commands::doctor::grouped(findings)
        .iter()
        .map(row)
        .collect()
}

fn row(finding: &Finding) -> FindingRow {
    FindingRow {
        component: finding.component.clone(),
        severity: finding.severity,
        summary: finding.summary.clone(),
        remedy: finding.remedy.clone(),
        detail: detail_lines(finding),
    }
}

/// The detail view: the same facts as the CLI's render, plus what pressing
/// `a` would do — a reader deciding whether to act must know which of the
/// three arms the keystroke lands in.
fn detail_lines(finding: &Finding) -> Vec<Line<'static>> {
    let mut body: Vec<Line<'static>> = Vec::new();
    let white = Style::new().fg(Color::White);
    let grey = Style::new().fg(Color::DarkGray);
    let header = Style::new().fg(Color::Yellow);
    let severity_style = if finding.severity == Severity::Broken {
        Style::new().fg(Color::Red)
    } else {
        Style::new().fg(Color::Yellow)
    };

    body.push(Line::styled(
        format!("{} · {}", finding.component, finding.severity.as_str()),
        severity_style,
    ));
    body.push(Line::styled(finding.summary.clone(), white));
    for line in finding.detail.lines() {
        body.push(Line::styled(line.to_string(), white));
    }
    body.push(Line::raw(""));
    match &finding.remedy {
        Some(remedy) => {
            body.push(Line::styled("remedy — a proposes it", header));
            for line in remedy.description.lines() {
                body.push(Line::styled(line.to_string(), white));
            }
            body.push(Line::styled(
                format!("run: {}", remedy.argv.join(" ")),
                white,
            ));
            match dispatch(remedy) {
                RemedyDispatch::DeepLink(_) => body.push(Line::styled(
                    "a opens that surface right here, as a modal",
                    grey,
                )),
                RemedyDispatch::Interactive => body.push(Line::styled(
                    "needs the real terminal — a suspends the TUI and hands it over",
                    grey,
                )),
                RemedyDispatch::Spawn => body.push(Line::styled(
                    "a confirms, runs it detached, and re-examines when it exits",
                    grey,
                )),
            }
        }
        None => body.push(Line::styled(
            "no remedy — this finding is the diagnosis, not a button",
            grey,
        )),
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::command::Command;

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

    fn remedy(argv: &[&str], needs_terminal: bool) -> Remedy {
        Remedy {
            description: "do the thing".into(),
            argv: argv.iter().map(|s| s.to_string()).collect(),
            needs_terminal,
        }
    }

    fn finding(component: &str, severity: Severity, remedy: Option<Remedy>) -> Finding {
        Finding {
            component: component.into(),
            severity,
            summary: format!("{component} is unwell"),
            detail: "the longer story".into(),
            remedy,
        }
    }

    /// The three arms, decided from the remedy alone. This is the test the
    /// task hinges on: the modal's key handler only routes what this returns.
    #[test]
    fn a_remedy_dispatches_to_a_deep_link_the_terminal_or_a_detached_spawn() {
        // A mecha subcommand with its own modal switches to it — even when
        // the CLI marked it `needs_terminal`, because on the command line
        // `outbox review` opens $EDITOR while here the modal is the surface.
        assert_eq!(
            dispatch(&remedy(&["mecha", "outbox", "review"], true)),
            RemedyDispatch::DeepLink(Command::Outbox)
        );
        assert_eq!(
            dispatch(&remedy(&["mecha", "frontdoor", "list"], false)),
            RemedyDispatch::DeepLink(Command::Frontdoor)
        );

        // The terminal arm: an OAuth flow needs a real keyboard.
        assert_eq!(
            dispatch(&remedy(&["mecha-mail", "auth", "personal"], true)),
            RemedyDispatch::Interactive
        );

        // Everything else is a confirmed detached spawn.
        assert_eq!(
            dispatch(&remedy(
                &["systemctl", "--user", "restart", "mecha-triggers.service"],
                false
            )),
            RemedyDispatch::Spawn
        );
        // A mecha verb without a modal is not a deep link.
        assert_eq!(
            dispatch(&remedy(&["mecha", "trigger", "run", "morning"], false)),
            RemedyDispatch::Spawn
        );
        // `mecha-mail import` is another binary and needs no terminal.
        assert_eq!(
            dispatch(&remedy(&["mecha-mail", "import", "google"], false)),
            RemedyDispatch::Spawn
        );
    }

    #[test]
    fn the_title_counts_broken_against_the_whole_and_names_the_keys() {
        let modal = DoctorModal::new(rows(vec![
            finding("mail", Severity::Broken, None),
            finding("outbox", Severity::Attention, None),
        ]));
        let title = modal.title();
        assert!(title.contains("2 finding(s)"), "{title}");
        assert!(title.contains("1 broken"), "{title}");
        for key in ["enter", "a act", "r re-examine", "esc"] {
            assert!(title.contains(key), "{key} missing from {title}");
        }

        let done = DoctorModal {
            status: Some("re-examined".into()),
            ..modal
        };
        assert!(done.title().contains("re-examined"));

        let healthy = DoctorModal::new(Vec::new());
        assert!(
            healthy.title().contains("nothing wrong"),
            "{}",
            healthy.title()
        );
    }

    /// The rows arrive grouped by component with broken first — the same
    /// order the CLI prints, because both go through `grouped`.
    #[test]
    fn rows_keep_a_component_together_and_lead_with_what_is_broken() {
        let rows = rows(vec![
            finding("outbox", Severity::Attention, None),
            finding("mail", Severity::Attention, None),
            finding("mail", Severity::Broken, None),
        ]);
        let order: Vec<(&str, Severity)> = rows
            .iter()
            .map(|r| (r.component.as_str(), r.severity))
            .collect();
        assert_eq!(
            order,
            vec![
                ("mail", Severity::Broken),
                ("mail", Severity::Attention),
                ("outbox", Severity::Attention),
            ]
        );
    }

    /// F9, failing on the old inline clamp: `clamp(1, height - 4)` panics
    /// (`min > max`) the moment the terminal is four rows or fewer. The
    /// computation degrades to a one-row box instead.
    #[test]
    fn the_list_height_never_panics_at_tiny_terminals() {
        for terminal_height in 0..=5u16 {
            for rows in 0..=3u16 {
                // The call itself is the assertion the old code fails: it
                // panicked here. The bounds check is the degradation story.
                let h = list_height(rows, terminal_height);
                let max_body = terminal_height.saturating_sub(4).max(1);
                assert!(
                    h >= 3 && h <= max_body + 2,
                    "rows={rows} terminal={terminal_height} -> {h}"
                );
            }
        }
        // The ordinary case is unchanged: content-sized plus the border.
        assert_eq!(list_height(5, 45), 7);
        // And a tall list still stops short of the screen.
        assert_eq!(list_height(200, 45), 43);
    }

    /// F5, failing on the old watch arm, which posted one notice at five
    /// minutes and dropped the watch — a zombie child and a lost outcome.
    /// The poll now keeps the watch alive with periodic notices and only
    /// gives up, killing and reaping, at a cap on the run-timeout scale.
    #[test]
    fn a_slow_remedy_is_kept_watched_with_notices_until_the_hard_cap() {
        use std::time::Duration;
        let min = |m: u64| Duration::from_secs(m * 60);

        assert_eq!(remedy_poll(min(4), 0), RemedyPoll::Wait);
        assert_eq!(remedy_poll(min(5), 0), RemedyPoll::Notice);
        // After the first notice the watch waits again rather than repeating
        // itself every second…
        assert_eq!(remedy_poll(min(6), 1), RemedyPoll::Wait);
        // …and the twelve-minute trigger run the old arm abandoned is still
        // being watched, two notices in.
        assert_eq!(remedy_poll(min(12), 2), RemedyPoll::Wait);
        assert_eq!(remedy_poll(min(15), 2), RemedyPoll::Notice);
        assert_eq!(remedy_poll(min(24), 4), RemedyPoll::Wait);
        // The hard cap ends it honestly.
        assert_eq!(remedy_poll(min(25), 4), RemedyPoll::Kill);
        assert_eq!(remedy_poll(min(60), 0), RemedyPoll::Kill);
    }

    /// F7: the modal opens at once in an examining state — the examination
    /// itself is a watched child, never a synchronous `.output()` on the
    /// event loop — and the parser is a pure function of the child's output.
    #[test]
    fn an_examining_modal_says_so_and_the_parser_answers_from_bytes() {
        let modal = DoctorModal::examining();
        assert!(modal.examining);
        assert!(modal.rows.is_empty());
        assert!(modal.title().contains("examining"), "{}", modal.title());

        // Exit 1's JSON parses the same as exit 0's — findings, not failure.
        let json = serde_json::to_string(&vec![finding("mail", Severity::Broken, None)]).unwrap();
        let rows = parse_examination(json.as_bytes(), b"").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].component, "mail");

        // Garbage is an error naming stderr's first line, not a guess.
        let err = parse_examination(b"not json", b"the store is torn\nmore")
            .unwrap_err()
            .to_string();
        assert!(err.contains("the store is torn"), "{err}");

        // A fresh install with no findings is healthy, not examining forever.
        let none = parse_examination(b"[]", b"").unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn the_selection_wraps_and_an_empty_list_does_not_panic() {
        let mut modal = DoctorModal::new(rows(vec![
            finding("mail", Severity::Broken, None),
            finding("outbox", Severity::Attention, None),
        ]));
        modal.move_by(-1);
        assert_eq!(modal.selected, 1);
        modal.move_by(1);
        assert_eq!(modal.selected, 0);

        let mut empty = DoctorModal::new(Vec::new());
        empty.move_by(1);
        assert_eq!(empty.selected, 0);
        assert!(empty.selected_row().is_none());
    }

    /// The detail says which arm `a` lands in — acting must not be a
    /// surprise, least of all when it is about to suspend the whole TUI.
    #[test]
    fn the_detail_names_the_remedy_and_which_arm_acting_takes() {
        let interactive = text(&detail_lines(&finding(
            "mail",
            Severity::Broken,
            Some(remedy(&["mecha-mail", "auth", "personal"], true)),
        )));
        assert!(
            interactive.contains("run: mecha-mail auth personal"),
            "{interactive}"
        );
        assert!(
            interactive.contains("needs the real terminal"),
            "{interactive}"
        );

        let deep = text(&detail_lines(&finding(
            "outbox",
            Severity::Attention,
            Some(remedy(&["mecha", "outbox", "review"], true)),
        )));
        assert!(deep.contains("opens that surface right here"), "{deep}");

        let spawn = text(&detail_lines(&finding(
            "systemd",
            Severity::Broken,
            Some(remedy(
                &["systemctl", "--user", "restart", "x.service"],
                false,
            )),
        )));
        assert!(spawn.contains("re-examines when it exits"), "{spawn}");

        let bare = text(&detail_lines(&finding("outbox", Severity::Attention, None)));
        assert!(bare.contains("no remedy"), "{bare}");
    }
}
