//! The `/docs` modal — Google Docs, Sheets and Slides without leaving the TUI.
//!
//! Ninth modal on the `/outbox` pattern, and it inherits that pattern's shape:
//! **read for display, and every mutation is a `mecha-docs …` child process.**
//! Nothing here reimplements a verb, so a thing the modal can do is a thing a
//! script or a trigger can do.
//!
//! What it is *for* is narrower than the name suggests, and deliberately so.
//! Under `drive.file` the interesting question is not "what is in my Drive" —
//! the model can already read and write anything in scope through the MCP
//! tools. It is **"how does a document get into scope at all"**, and the
//! answer used to be a CLI command that prints a URL and then blocks on stdin,
//! which on a headless box means either an SSH tunnel to port 8765 or copying
//! a 400-character URL out of one terminal and a redirect address back into
//! it. That is the kludge. `mecha-docs pick --url` / `--redirect` split the
//! browser leg into two commands that can run minutes apart, and this modal is
//! what drives them.
//!
//! Three decisions:
//!
//! - **Every network call is off the event loop.** Listing the scope is a
//!   Drive request and finishing a pick is a token exchange; either on the
//!   event loop freezes the interface at exactly the moment someone is waiting
//!   for it. Answers come back through a `Watch`, the same way `/mail` reads a
//!   thread.
//! - **The picking is never done here.** The chooser is Google's own, in the
//!   user's own browser, outside this process and outside the model's context.
//!   That is the whole safety argument for `drive.file` — see the Documents
//!   section of CLAUDE.md — and a modal that offered to pick *for* you would
//!   be an argument for widening the scope. All this does is carry a URL out
//!   and an address back.
//! - **`enter` writes a reference into the message box rather than doing
//!   anything.** The point of seeing the list is to be able to *ask about* a
//!   document, and the id is the part a person cannot retype. It goes into the
//!   input for the user to finish and send — never onto the wire, because
//!   composing a prompt on someone's behalf is not this modal's job.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use super::{centered, list_height_reserving};

/// One in-scope file, flattened for display.
pub struct DocRow {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub modified: String,
}

impl DocRow {
    /// The document's own address, which is what a person wants when they are
    /// going to open it themselves.
    pub fn url(&self) -> String {
        match self.kind.as_str() {
            "doc" => format!("https://docs.google.com/document/d/{}/edit", self.id),
            "sheet" => format!("https://docs.google.com/spreadsheets/d/{}/edit", self.id),
            "slides" => format!("https://docs.google.com/presentation/d/{}/edit", self.id),
            "folder" => format!("https://drive.google.com/drive/folders/{}", self.id),
            _ => format!("https://drive.google.com/file/d/{}/view", self.id),
        }
    }

    /// What `enter` puts in the message box. The name is for the person, the
    /// id is for the model — a `docs_*` tool takes the id and nothing else,
    /// and a name is exactly the kind of thing a model will confidently
    /// hallucinate an id for.
    pub fn reference(&self) -> String {
        format!("the {} \"{}\" (id {})", self.kind, self.name, self.id)
    }
}

/// A browser leg in flight: the URL the user has to open, and the address
/// they paste back.
pub struct Pick {
    pub url: String,
    pub buffer: String,
    /// Byte offset into `buffer`, so editing a pasted address is not
    /// append-only.
    pub cursor: usize,
    /// The exchange is running. A second `enter` while it is would start a
    /// second one against the same one-use code.
    pub working: bool,
    /// Show the link and nothing else, for selecting it with the mouse.
    ///
    /// A bordered box is unselectable for a string this long: the URL is hard
    /// wrapped over four rows, and a drag across them takes the `│` at each
    /// end of every row with it, so what lands on the clipboard is not a URL.
    /// The bare view puts each row at column 0 with nothing else on the line,
    /// which is the only shape a terminal's own selection can copy correctly.
    pub bare: bool,
}

pub struct DocsModal {
    /// Which grant is being shown. Accounts are directories under
    /// `~/.mecha/docs/`; the listing *is* the list, as it is for mail.
    pub account: String,
    pub accounts: Vec<String>,
    pub rows: Vec<DocRow>,
    pub selected: usize,
    pub status: Option<String>,
    pub pick: Option<Pick>,
    /// A list or an exchange is in flight. Shown, because a modal that looks
    /// empty and a modal that is still loading are different facts.
    pub loading: bool,
    pub help: bool,
}

impl DocsModal {
    pub fn new(account: String, accounts: Vec<String>) -> Self {
        DocsModal {
            account,
            accounts,
            rows: Vec::new(),
            selected: 0,
            status: None,
            pick: None,
            loading: true,
            help: false,
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let n = self.rows.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(n) as usize;
    }

    /// Keep the selection on screen when the list is taller than the modal.
    ///
    /// The same four lines as every sibling modal, deliberately — this started
    /// out windowing the rows with `.skip().take()` instead, which works and
    /// is a ninth way of doing an eighth thing. A new modal is written by
    /// copying whichever one is nearest, so the one worth being nearest is
    /// the one everything else already does. The status strip is the last row
    /// of `lines`, so it scrolls with them and the box never shows a stale
    /// hint pinned under rows it does not belong to.
    fn list_scroll(&self, visible: u16) -> u16 {
        let visible = visible.max(1) as usize;
        (self.selected + 1).saturating_sub(visible) as u16
    }

    pub fn current(&self) -> Option<&DocRow> {
        self.rows.get(self.selected)
    }

    /// The next account in the list, wrapping. `None` when there is nothing to
    /// switch to, so the key does nothing rather than pretending.
    pub fn next_account(&self) -> Option<String> {
        if self.accounts.len() < 2 {
            return None;
        }
        let at = self.accounts.iter().position(|a| *a == self.account)?;
        Some(self.accounts[(at + 1) % self.accounts.len()].clone())
    }

    /// Install the answer from `mecha-docs list --json`.
    pub fn install(&mut self, json: &str) {
        #[derive(serde::Deserialize)]
        struct Raw {
            id: String,
            #[serde(default)]
            name: String,
            #[serde(rename = "mimeType", default)]
            mime_type: String,
            #[serde(rename = "modifiedTime", default)]
            modified_time: Option<String>,
        }
        self.loading = false;
        let files: Vec<Raw> = match serde_json::from_str(json) {
            Ok(f) => f,
            Err(e) => {
                self.status = Some(format!("could not read the listing: {e}"));
                return;
            }
        };
        self.rows = files
            .into_iter()
            .map(|f| DocRow {
                kind: kind_of(&f.mime_type).to_string(),
                id: f.id,
                name: f.name,
                // The date is what a person recognises a document by when two
                // have similar names; the time of day is noise at this width.
                modified: f
                    .modified_time
                    .unwrap_or_default()
                    .split('T')
                    .next()
                    .unwrap_or_default()
                    .to_string(),
            })
            .collect();
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }
}

/// The same mapping `mecha-docs` prints, kept here rather than depended on:
/// this crate has no `mecha-mail` dependency and gaining one to name four
/// strings would be the wrong trade — `mecha-cli` reaches the documents
/// surface as a *binary*, which is what lets the two be installed and
/// upgraded apart.
fn kind_of(mime: &str) -> &'static str {
    match mime {
        "application/vnd.google-apps.document" => "doc",
        "application/vnd.google-apps.spreadsheet" => "sheet",
        "application/vnd.google-apps.presentation" => "slides",
        "application/vnd.google-apps.folder" => "folder",
        _ => "file",
    }
}

/// Ask the terminal to put `text` on the *user's* clipboard (OSC 52).
///
/// This is the one thing here that genuinely solves "kludgy on a remote
/// server": the escape travels back over the same SSH connection the screen
/// does, so the clipboard it lands on is the laptop's, not the server's. There
/// is no reply to read, so nothing can confirm it worked — hence the hedged
/// wording wherever this is reported. Terminals differ (kitty, iTerm2 and
/// WezTerm allow it; tmux needs `set -g set-clipboard on`), and a terminal
/// that refuses simply does nothing, which is why the URL stays on screen to
/// be selected by hand as well.
pub fn clipboard_escape(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

/// Standard base64, hand-rolled rather than adding a dependency edge for one
/// escape sequence. No line breaks: OSC 52 payloads are one string.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

impl DocsModal {
    pub fn draw(&self, frame: &mut Frame) {
        if self.help {
            self.draw_help(frame);
            return;
        }
        if let Some(pick) = &self.pick {
            self.draw_pick(frame, pick);
            return;
        }

        let area = frame.area();
        let rows = list_height_reserving(self.rows.len() as u16, area.height, 1);
        let box_area = centered(area, area.width.saturating_sub(6).min(110), rows);
        frame.render_widget(Clear, box_area);

        let mut lines: Vec<Line> = Vec::new();
        if self.rows.is_empty() {
            lines.push(Line::styled(
                if self.loading {
                    "  loading…"
                } else {
                    "  nothing in scope yet — press p to pick a document"
                },
                Style::new().fg(Color::DarkGray),
            ));
        }
        for (i, row) in self.rows.iter().enumerate() {
            let here = i == self.selected;
            let marker = if here { "▸ " } else { "  " };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{marker}{:<7} ", row.kind),
                    Style::new().fg(if here { Color::Cyan } else { Color::DarkGray }),
                ),
                Span::styled(
                    format!("{:<48} ", clip(&row.name, 48)),
                    if here {
                        Style::new().fg(Color::White).bold()
                    } else {
                        Style::new().fg(Color::White)
                    },
                ),
                Span::styled(row.modified.clone(), Style::new().fg(Color::DarkGray)),
            ]));
        }
        lines.push(Line::styled(
            match &self.status {
                Some(s) => format!("  {s}"),
                None => "  enter insert · p pick · y copy link · r refresh · a account · ? keys"
                    .to_string(),
            },
            Style::new().fg(Color::DarkGray),
        ));

        frame.render_widget(
            Paragraph::new(lines)
                .scroll((self.list_scroll(box_area.height.saturating_sub(2)), 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::new().fg(Color::Cyan))
                        .title(format!(
                            " docs · {} · {} in scope{} ",
                            self.account,
                            self.rows.len(),
                            if self.loading { " · loading…" } else { "" }
                        )),
                ),
            box_area,
        );
    }

    /// The picking pane: the URL to open, and the address that comes back.
    ///
    /// The URL is on screen in full rather than behind a "copied!" — an OSC 52
    /// write has no reply, so nothing here can know whether the clipboard
    /// took it, and a pane that hid the URL on that assumption would strand
    /// anyone whose terminal refuses. It is the fallback and the record.
    fn draw_pick(&self, frame: &mut Frame, pick: &Pick) {
        if pick.bare {
            self.draw_bare(frame, pick);
            return;
        }
        let area = frame.area();
        let width = area.width.saturating_sub(6).min(100);
        // Four lines of URL is enough for the ~420 characters this one runs to
        // at any usable width; the rest of the box is fixed furniture.
        let height = 16u16.min(area.height);
        let box_area = centered(area, width, height);
        frame.render_widget(Clear, box_area);

        let dim = Style::new().fg(Color::DarkGray);
        let mut lines: Vec<Line> = vec![
            Line::styled(
                "Open this in any browser, on any machine — it does not have to be this one:",
                Style::new().fg(Color::White),
            ),
            Line::raw(""),
        ];
        for chunk in wrap_hard(&pick.url, width.saturating_sub(4) as usize, 5) {
            lines.push(Line::styled(chunk, Style::new().fg(Color::Cyan)));
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "It finishes on a 127.0.0.1 address that fails to load. That is expected —",
            dim,
        ));
        lines.push(Line::styled(
            "copy the whole address out of the bar and paste it here:",
            dim,
        ));
        lines.push(Line::styled(
            format!(
                "  {}",
                if pick.buffer.is_empty() {
                    "…".to_string()
                } else {
                    clip(&pick.buffer, width.saturating_sub(4) as usize)
                }
            ),
            Style::new().fg(Color::White),
        ));
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            match (&self.status, pick.working) {
                (_, true) => "  exchanging…".to_string(),
                (Some(s), _) => format!("  {s}"),
                (None, _) => "  s select it · o open it here · y copy · enter finish · esc cancel"
                    .to_string(),
            },
            dim,
        ));

        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(Color::Yellow))
                    .title(format!(" pick a document · {} ", self.account)),
            ),
            box_area,
        );
    }

    /// The link, alone, at column 0.
    ///
    /// No border and no other text on any row the URL occupies, because a
    /// terminal's selection is rectangular over what is on screen: anything
    /// sharing those rows is copied too. The hint sits below the blank line
    /// after the link, far enough that an over-long drag catches nothing.
    /// The mouse is the terminal's while a pick is up (`wants_the_mouse_back`
    /// in the TUI's own module), so a drag here selects rather than being
    /// swallowed as a mouse event.
    fn draw_bare(&self, frame: &mut Frame, pick: &Pick) {
        let area = frame.area();
        frame.render_widget(Clear, area);

        let mut lines: Vec<Line> = Vec::new();
        // The full width, and a generous row cap: this view exists so the
        // whole link is reachable, and a truncation here would be the exact
        // failure it was written to fix.
        for chunk in wrap_hard(&pick.url, area.width.max(1) as usize, 40) {
            lines.push(Line::styled(chunk, Style::new().fg(Color::Cyan)));
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "drag to select · your terminal's own copy · s back · esc cancel",
            Style::new().fg(Color::DarkGray),
        ));
        frame.render_widget(Paragraph::new(lines), area);
    }

    fn draw_help(&self, frame: &mut Frame) {
        let keys = [
            ("↑ ↓ / j k", "move"),
            ("enter", "put a reference to it in the message box"),
            ("p", "pick a document: opens Google's own chooser"),
            ("y", "copy its link to your clipboard (OSC 52)"),
            (
                "s",
                "while picking: the link alone, to select with the mouse",
            ),
            ("o", "while picking: open the link on this machine"),
            ("r", "re-read what is in scope"),
            ("a", "next account"),
            ("esc q", "close"),
        ];
        let mut lines: Vec<Line> = keys
            .iter()
            .map(|(k, what)| {
                Line::from(vec![
                    Span::styled(format!("  {k:<12}"), Style::new().fg(Color::Cyan)),
                    Span::styled(*what, Style::new().fg(Color::White)),
                ])
            })
            .collect();
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            "  Only files mecha created, or that you handed it with `p`, are",
            Style::new().fg(Color::DarkGray),
        ));
        lines.push(Line::styled(
            "  reachable at all — that is the drive.file scope, and nothing said",
            Style::new().fg(Color::DarkGray),
        ));
        lines.push(Line::styled(
            "  inside a run can widen it.",
            Style::new().fg(Color::DarkGray),
        ));
        let area = centered(
            frame.area(),
            72,
            (lines.len() as u16 + 2).min(frame.area().height),
        );
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(Color::Cyan))
                    .title(" docs · keys · any key to close "),
            ),
            area,
        );
    }
}

/// Break a string that has no spaces in it — a URL — into rows, keeping at
/// most `max` of them and marking the cut. Truncating in silence is how a
/// person copies three quarters of an authorization URL.
fn wrap_hard(s: &str, width: usize, max: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out: Vec<String> = Vec::new();
    let mut row = String::new();
    for c in s.chars() {
        if row.chars().count() == width {
            out.push(std::mem::take(&mut row));
            if out.len() == max {
                out.push(format!(
                    "… +{} more characters — press y to copy it all",
                    s.chars().count() - out.len() * width
                ));
                return out;
            }
        }
        row.push(c);
    }
    if !row.is_empty() {
        out.push(row);
    }
    out
}

fn clip(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        return s.to_string();
    }
    s.chars().take(width.saturating_sub(1)).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    fn modal() -> DocsModal {
        let mut m = DocsModal::new("personal".into(), vec!["personal".into(), "work".into()]);
        m.install(
            r#"[{"id":"1abc","name":"Psych60_F2026","mimeType":"application/vnd.google-apps.spreadsheet","modifiedTime":"2026-08-20T14:12:44.487Z"},
                {"id":"2def","name":"Py-FEAT v2.0 Manuscript","mimeType":"application/vnd.google-apps.document","modifiedTime":"2026-08-18T23:24:03.683Z"}]"#,
        );
        m
    }

    fn frame_text(m: &DocsModal, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| m.draw(f)).unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_list_names_the_kind_the_name_and_the_day_it_moved() {
        let text = frame_text(&modal(), 100, 20);
        assert!(text.contains("sheet"), "{text}");
        assert!(text.contains("Psych60_F2026"), "{text}");
        assert!(text.contains("2026-08-20"), "{text}");
        assert!(text.contains("2 in scope"), "{text}");
    }

    #[test]
    fn an_empty_scope_says_how_to_fill_it_rather_than_looking_broken() {
        let mut m = DocsModal::new("personal".into(), vec!["personal".into()]);
        m.install("[]");
        let text = frame_text(&m, 100, 20);
        assert!(text.contains("nothing in scope yet"), "{text}");
        assert!(text.contains("press p to pick"), "{text}");
    }

    #[test]
    fn loading_and_empty_are_different_facts() {
        // "Nothing here" and "nothing yet" read the same on screen unless the
        // modal says so — the null-run shape, one layer up.
        let m = DocsModal::new("personal".into(), vec!["personal".into()]);
        let text = frame_text(&m, 100, 20);
        assert!(text.contains("loading"), "{text}");
        assert!(!text.contains("nothing in scope"), "{text}");
    }

    #[test]
    fn a_reference_carries_the_id_because_that_is_what_a_tool_takes() {
        let m = modal();
        let r = m.current().unwrap().reference();
        assert!(r.contains("1abc"), "{r}");
        assert!(r.contains("Psych60_F2026"), "{r}");
        assert!(r.contains("sheet"), "{r}");
    }

    #[test]
    fn each_kind_gets_the_address_that_actually_opens_it() {
        let m = modal();
        assert!(m.rows[0].url().contains("/spreadsheets/d/1abc"));
        assert!(m.rows[1].url().contains("/document/d/2def"));
    }

    #[test]
    fn the_pick_pane_shows_the_whole_url_or_says_it_did_not() {
        let mut m = modal();
        m.pick = Some(Pick {
            url: format!(
                "https://accounts.google.com/o/oauth2/v2/auth?{}",
                "x".repeat(600)
            ),
            buffer: String::new(),
            cursor: 0,
            working: false,
            bare: false,
        });
        let text = frame_text(&m, 100, 24);
        assert!(text.contains("Open this in any browser"), "{text}");
        // Too long to show in full at this size, and it says so rather than
        // letting someone copy three quarters of an authorization URL.
        assert!(text.contains("more characters"), "{text}");
    }

    fn picking(url: &str) -> DocsModal {
        let mut m = modal();
        m.pick = Some(Pick {
            url: url.to_string(),
            buffer: String::new(),
            cursor: 0,
            working: false,
            bare: false,
        });
        m
    }

    #[test]
    fn the_pick_pane_says_the_link_can_be_selected_and_opened() {
        // The pane used to offer `y` and nothing else, and `y` is an OSC 52
        // write with no reply — so a terminal that refuses it left a person
        // looking at a link with no way to reach it.
        let text = frame_text(
            &picking("https://accounts.google.com/o/oauth2/v2/auth?x=1"),
            100,
            24,
        );
        assert!(text.contains("s select it"), "{text}");
        assert!(text.contains("o open it here"), "{text}");
    }

    #[test]
    fn the_bare_view_is_the_link_and_nothing_a_selection_could_catch() {
        // The point of the view: every row the URL occupies holds the URL and
        // nothing else, so a drag across four wrapped rows copies a URL rather
        // than a URL with a border character every eightieth column.
        let url = format!(
            "https://accounts.google.com/o/oauth2/v2/auth?client_id=949095882298&{}",
            "x".repeat(300)
        );
        let mut m = picking(&url);
        m.pick.as_mut().unwrap().bare = true;
        let text = frame_text(&m, 100, 24);
        let rows: Vec<&str> = text.lines().collect();
        assert!(rows[0].starts_with("https://accounts.google.com"), "{text}");
        assert!(
            !text.contains('│'),
            "a border shares a row with the link: {text}"
        );
        assert!(!text.contains("Open this in any browser"), "{text}");
        // And all of it is there: this view exists so the whole link is
        // reachable, and a truncation would be the failure it was written for.
        let on_screen: String = rows
            .iter()
            .take_while(|r| !r.trim().is_empty())
            .map(|r| r.trim_end())
            .collect();
        assert_eq!(on_screen, url, "{text}");
    }

    #[test]
    fn a_long_list_scrolls_to_keep_the_selection_visible() {
        // The rule every sibling modal follows, and it is not cosmetic here
        // either: `enter` on a row you cannot see puts the wrong document id
        // in the message box.
        let mut m = DocsModal::new("personal".into(), vec!["personal".into()]);
        m.rows = (0..60)
            .map(|i| DocRow {
                id: format!("id{i:02}"),
                name: format!("document{i:02}"),
                kind: "doc".into(),
                modified: "2026-08-20".into(),
            })
            .collect();
        m.selected = 55;
        let text = frame_text(&m, 100, 24);
        assert!(
            text.contains("document55"),
            "the selection is off screen: {text}"
        );
        assert!(
            !text.contains("document00"),
            "the head has scrolled off: {text}"
        );
    }

    #[test]
    fn base64_matches_the_standard_including_its_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn the_clipboard_escape_is_the_osc_52_a_terminal_expects() {
        let esc = clipboard_escape("hi");
        assert_eq!(esc, "\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn accounts_cycle_and_a_lone_account_has_nowhere_to_go() {
        let m = modal();
        assert_eq!(m.next_account().as_deref(), Some("work"));
        let solo = DocsModal::new("personal".into(), vec!["personal".into()]);
        assert!(solo.next_account().is_none());
    }

    #[test]
    fn a_tiny_terminal_shrinks_the_list_rather_than_panicking() {
        // The `list_height` rule: the assertion is the draw itself.
        for height in 1..8u16 {
            let _ = frame_text(&modal(), 40, height);
        }
    }
}
