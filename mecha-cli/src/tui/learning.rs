//! The `/learning` modal: what mecha has been taught, at all three stages.
//!
//! **One store, three stages, and they were reviewable in none of them.**
//! `reflections.jsonl` had no reader at all; `learned.toml` had `mecha rules`
//! and no UI; a rule proposal was reviewable only through `/queues`, where the
//! whole set is one row and the only available objection is reject-all. So the
//! pipeline that writes into every future prompt's cached prefix was the least
//! visible thing in the system.
//!
//! The three panes are the three stages, in the order a lesson passes through
//! them:
//!
//! | pane | what it holds | what you can do |
//! |---|---|---|
//! | **reflections** | one lesson per intervention, before consolidation | edit · drop · restore |
//! | **rules** | what a run actually carries, with its ledger tallies | retire · restore |
//! | **proposals** | a rewritten rule set waiting on a decision | accept · reject |
//!
//! **Edit is the important verb and it lives at the first stage**, which is
//! the argument for the whole modal. A rule is a *consolidation* of several
//! lessons, so by the time a proposal exists the thing to disagree with has
//! been merged with four others and rewritten; disagreeing there costs the
//! four good ones. At the reflection it costs nothing and says exactly what
//! was wrong — and because an edited lesson is the owner's own words, it also
//! *promotes* a reflection the provenance gate had excluded.
//!
//! **Every mutation is a `mecha …` child process**, on `/triggers`' rule: one
//! implementation per verb, and nothing the modal can do that the command line
//! cannot. `/queues` keeps its rule-proposals row and hands off here, for the
//! reason it already hands off to `/outbox` — the store with the affordances
//! owns them, and a second copy is a second thing to keep correct.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Reflections,
    Rules,
    Proposals,
}

impl Pane {
    pub const ALL: [Pane; 3] = [Pane::Reflections, Pane::Rules, Pane::Proposals];

    pub fn label(self) -> &'static str {
        match self {
            Pane::Reflections => "reflections",
            Pane::Rules => "rules",
            Pane::Proposals => "proposals",
        }
    }

    /// The `mecha` subcommand this pane's rows come from and act through.
    pub fn verb(self) -> &'static str {
        match self {
            Pane::Reflections => "reflections",
            Pane::Rules => "rules",
            Pane::Proposals => "proposals",
        }
    }

    pub fn next(self) -> Pane {
        match self {
            Pane::Reflections => Pane::Rules,
            Pane::Rules => Pane::Proposals,
            Pane::Proposals => Pane::Reflections,
        }
    }

    pub fn prev(self) -> Pane {
        self.next().next()
    }

    /// What the key strip offers here. Written per pane rather than as one
    /// line, because a key that does nothing in the pane you are looking at is
    /// worse than no key: it is a promise.
    fn keys(self) -> &'static str {
        match self {
            Pane::Reflections => {
                "j/k · Enter read · e edit the lesson · d drop · u restore · tab pane · Esc"
            }
            Pane::Rules => "j/k · Enter read · x retire · u restore · tab pane · Esc",
            Pane::Proposals => "j/k · Enter read · a accept (applies it) · r reject · tab · Esc",
        }
    }
}

/// One row, in the shape all three stages answer in.
///
/// Deliberately one type. The three stores have different records, and the
/// modal's job is the same in each: name it, say what state it is in, say why
/// it is in that state, and hand a decision to a child process. Three bespoke
/// row types would be three renderings to keep in step.
#[derive(Debug, Clone)]
pub struct Row {
    pub id: String,
    /// The lesson, the rule, or the proposal's one-line summary.
    pub title: String,
    /// Domain, or the proposal's status.
    pub tag: String,
    /// Why it is excluded, retired, or waiting — the sentence a decision rests
    /// on. `None` when there is nothing in its way.
    pub note: Option<String>,
    /// Dropped, retired, or resolved: shown, and shown as past.
    pub spent: bool,
    /// The owner has already touched this one — an edited lesson, a user rule.
    pub mine: bool,
}

pub struct LearningModal {
    pub pane: Pane,
    pub rows: Vec<Row>,
    pub selected: usize,
    /// The selected record in full, from `<verb> show`.
    pub detail: Option<String>,
    pub detail_scroll: u16,
    pub status: Option<String>,
    pub help: bool,
    /// Set while a child is running, so a second keypress cannot start a
    /// second one against the same record.
    pub busy: bool,
}

impl LearningModal {
    pub fn new(pane: Pane, rows: Vec<Row>) -> Self {
        LearningModal {
            pane,
            rows,
            selected: 0,
            detail: None,
            detail_scroll: 0,
            status: None,
            help: false,
            busy: false,
        }
    }

    pub fn selected(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    /// Move the cursor, and drop the detail with it.
    ///
    /// A detail carried onto another row is a document about something else,
    /// and the scroll offset inside it is a position in that other document —
    /// the `/tasks` detail_scroll lesson, which cost a keypress landing on the
    /// wrong record.
    pub fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last as isize) as usize;
        self.detail = None;
        self.detail_scroll = 0;
    }

    /// Switch pane. The cursor goes to the top: a different pane is a
    /// different list, and the next keypress may be `d`.
    pub fn set_pane(&mut self, pane: Pane, rows: Vec<Row>) {
        self.pane = pane;
        self.rows = rows;
        self.selected = 0;
        self.detail = None;
        self.detail_scroll = 0;
    }

    /// Re-find the cursor by **id** after a reload.
    ///
    /// Acting on a row is also what reorders or removes it — a dropped
    /// reflection leaves the default listing, an accepted proposal changes
    /// status — so an index carried across a reload names a different record
    /// to the next keypress. `/outbox`'s hidden-items toggle learned this as
    /// an edge case; here it is the common path, since every action reloads.
    pub fn reload(&mut self, rows: Vec<Row>) {
        let was = self.selected().map(|r| r.id.clone());
        self.rows = rows;
        self.selected = was
            .and_then(|id| self.rows.iter().position(|r| r.id == id))
            .unwrap_or(0)
            .min(self.rows.len().saturating_sub(1));
        self.detail = None;
        self.detail_scroll = 0;
    }

    fn title(&self) -> String {
        let n = self.rows.len();
        let mine = self.rows.iter().filter(|r| r.mine).count();
        let tabs: Vec<String> = Pane::ALL
            .iter()
            .map(|p| {
                if *p == self.pane {
                    format!("[{}]", p.label())
                } else {
                    format!(" {} ", p.label())
                }
            })
            .collect();
        match mine {
            0 => format!(" learning · {} · {n} ", tabs.join("")),
            _ => format!(" learning · {} · {n}, {mine} yours ", tabs.join("")),
        }
    }

    fn list_lines(&self) -> Vec<Line<'static>> {
        if self.rows.is_empty() {
            return vec![Line::styled(
                match self.pane {
                    Pane::Reflections => "  no reflections yet — `mecha reflect` mines them",
                    Pane::Rules => "  no rules yet — `mecha learn` creates them",
                    Pane::Proposals => "  nothing waiting",
                },
                Style::new().fg(Color::DarkGray),
            )];
        }
        let mut out = Vec::new();
        for (i, r) in self.rows.iter().enumerate() {
            let here = i == self.selected;
            let body = if r.spent {
                Style::new().fg(Color::DarkGray).add_modifier(Modifier::DIM)
            } else if here {
                Style::new().fg(Color::White).bold()
            } else {
                Style::new().fg(Color::White)
            };
            out.push(Line::from(vec![
                Span::styled(if here { "▸ " } else { "  " }, Style::new().fg(Color::Cyan)),
                // One glyph for the owner's own mark, because it is the thing
                // most worth telling apart at a glance: what mecha decided,
                // and what you did about it.
                Span::styled(
                    if r.mine { "✎ " } else { "  " },
                    Style::new().fg(Color::Green),
                ),
                Span::styled(
                    format!("{:<10} ", truncate(&r.tag, 10)),
                    Style::new().fg(Color::DarkGray),
                ),
                Span::styled(truncate(&r.title, 92), body),
            ]));
            // Only under the cursor: it is the sentence the decision rests on
            // and it is long, so every row's would hide the one being decided.
            if here {
                if let Some(note) = &r.note {
                    out.push(Line::styled(
                        format!("      └ {note}"),
                        Style::new().fg(Color::Yellow),
                    ));
                }
            }
        }
        out
    }

    pub fn draw(&self, frame: &mut Frame) {
        if self.help {
            self.draw_help(frame);
            return;
        }
        let strip_text = format!("  {}", self.pane.keys());
        let width = 122u16.min(frame.area().width);
        let body = match &self.detail {
            // Wrapped, for `/queues`' reason: this is the text that goes into
            // every future run, and a sentence clipped at the box edge is a
            // decision taken on an unread field.
            Some(text) => super::queues::wrapped(text, width.saturating_sub(4)),
            None => self.list_lines(),
        };

        let reserved = 1 + u16::from(self.status.is_some());
        let height = super::list_height_reserving(body.len() as u16, frame.area().height, reserved);
        let area = super::centered(frame.area(), width, height);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(Color::Cyan))
            .title(self.title());
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if inner.height == 0 {
            return;
        }
        frame.render_widget(
            Paragraph::new(Line::styled(strip_text, Style::new().fg(Color::Cyan)))
                .wrap(Wrap { trim: false }),
            Rect { height: 1, ..inner },
        );
        let mut used = 1;
        if let Some(s) = &self.status {
            if used < inner.height {
                frame.render_widget(
                    Paragraph::new(Line::styled(
                        format!("  {s}"),
                        Style::new().fg(Color::Yellow),
                    )),
                    Rect {
                        y: inner.y + used,
                        height: 1,
                        ..inner
                    },
                );
                used += 1;
            }
        }
        let list = Rect {
            y: inner.y + used,
            height: inner.height.saturating_sub(used),
            ..inner
        };
        let scroll = match self.detail.is_some() {
            true => self.detail_scroll,
            false => self.list_scroll(list.height),
        };
        frame.render_widget(Paragraph::new(body).scroll((scroll, 0)), list);
    }

    /// Keep the cursor on screen without moving it.
    fn list_scroll(&self, height: u16) -> u16 {
        // Two lines per selected row: the row and its note.
        let cursor = self.selected as u16;
        cursor.saturating_sub(height.saturating_sub(3))
    }

    fn draw_help(&self, frame: &mut Frame) {
        let text = "\
  /learning — the three stages a lesson passes through

  reflections   one lesson per intervention, before anything merges them.
                This is where disagreeing is cheap: a rule is a consolidation
                of several, so objecting at the proposal costs the good ones.

                e   rewrite the lesson in your own words. That is a provenance
                    promotion, not a text change — a lesson you typed is yours,
                    so one the gate excluded becomes learnable. What was
                    happening is withheld, because that is the field that held
                    the third-party text.
                d   refuse it. Kept as evidence, never a candidate again.
                u   undo a drop.

  rules         what a run actually carries, with its ledger tallies.
                User rules are marked and are not on trial.

                x   retire. A flag, never a deletion: the rule stays in the
                    file and the learner is told it was measured harmful.
                u   restore.

  proposals     a rewritten rule set waiting on a decision.

                a   accept — applies it to the live rules.
                r   reject — the reason is recorded and mined.

  tab / shift-tab   move between panes      Enter   read one in full
  Esc               back, then close        ?       this
";
        let lines: Vec<Line<'static>> = text
            .lines()
            .map(|l| Line::styled(l.to_string(), Style::new().fg(Color::White)))
            .collect();
        let width = 100u16.min(frame.area().width);
        let height = super::list_height(lines.len() as u16, frame.area().height);
        let area = super::centered(frame.area(), width, height);
        frame.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::new().fg(Color::Cyan))
            .title(" learning · help ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    match s.chars().count() > max {
        true => format!("{}…", s.chars().take(max - 1).collect::<String>()),
        false => s,
    }
}

// ─── JSON in ─────────────────────────────────────────────────────────────────

/// Parse one pane's `list --json`.
///
/// One function for all three, because the three commands were given the same
/// listing shape on purpose — `id`, `title`, and whatever each stage calls its
/// state. A pane that answered a shape of its own would be a fourth renderer.
pub fn rows_from_json(pane: Pane, text: &str) -> anyhow::Result<Vec<Row>> {
    let v: serde_json::Value = serde_json::from_str(text)?;
    let arr = v.as_array().cloned().unwrap_or_default();
    Ok(arr
        .iter()
        .map(|r| {
            let s = |k: &str| r.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let b = |k: &str| r.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
            match pane {
                Pane::Reflections => Row {
                    id: s("id"),
                    title: s("title"),
                    tag: s("domain"),
                    note: r
                        .get("blocked")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    spent: b("dropped"),
                    mine: b("edited"),
                },
                Pane::Rules => {
                    let note = if b("user") {
                        Some("yours — never tallied, never retired".to_string())
                    } else {
                        match (
                            r.get("observations").and_then(|v| v.as_u64()),
                            r.get("attributed_regressions").and_then(|v| v.as_u64()),
                        ) {
                            // Absent is not zero: a rule no probe has ever
                            // reached is not a rule that passed.
                            (None, _) | (Some(0), _) => {
                                Some("never validated — no probe has reached it".into())
                            }
                            (Some(n), Some(bad)) if bad > 0 => {
                                Some(format!("{n} probe(s), {bad} attributed regression(s)"))
                            }
                            (Some(n), _) => Some(format!("{n} probe(s), none attributed")),
                        }
                    };
                    Row {
                        id: s("id"),
                        title: s("title"),
                        tag: s("domain"),
                        note: match b("retired") {
                            true => Some(format!(
                                "retired{}",
                                r.get("retired_reason")
                                    .and_then(|v| v.as_str())
                                    .map(|w| format!(" — {w}"))
                                    .unwrap_or_default()
                            )),
                            false => note,
                        },
                        spent: b("retired"),
                        mine: b("user"),
                    }
                }
                Pane::Proposals => Row {
                    id: s("id"),
                    title: s("title"),
                    // `proposals list --json` calls these `kind` and
                    // `detail` — there is no `status` key at all, so reading
                    // one made `tag` blank and `spent` true for every row:
                    // `list` already filters to `status == "pending"`, so
                    // nothing it can return is spent.
                    tag: s("kind"),
                    note: r.get("detail").and_then(|v| v.as_str()).map(str::to_string),
                    spent: false,
                    mine: false,
                },
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modal(n: usize) -> LearningModal {
        LearningModal::new(
            Pane::Reflections,
            (0..n)
                .map(|i| Row {
                    id: format!("r{i}"),
                    title: format!("lesson {i}"),
                    tag: "behavior".into(),
                    note: None,
                    spent: false,
                    mine: false,
                })
                .collect(),
        )
    }

    #[test]
    fn the_cursor_stays_inside_the_list() {
        let mut m = modal(3);
        m.move_by(-1);
        assert_eq!(m.selected, 0);
        m.move_by(9);
        assert_eq!(m.selected, 2);
    }

    /// Every action reloads, and acting on a row is what removes it from the
    /// default listing — so an index carried across would name a different
    /// record to the next keypress.
    #[test]
    fn a_reload_re_finds_the_cursor_by_id() {
        let mut m = modal(4);
        m.move_by(2);
        assert_eq!(m.selected().unwrap().id, "r2");

        let mut left = m.rows.clone();
        left.remove(0); // r0 was dropped and left the listing
        m.reload(left);
        assert_eq!(
            m.selected().unwrap().id,
            "r2",
            "the cursor followed the record, not the position"
        );
    }

    #[test]
    fn a_reload_that_loses_the_row_lands_somewhere_real() {
        let mut m = modal(3);
        m.move_by(2);
        m.reload(vec![m.rows[0].clone()]);
        assert_eq!(m.selected, 0);

        m.reload(Vec::new());
        assert_eq!(m.selected, 0);
        assert!(m.selected().is_none());
    }

    /// A detail is a document about one record. Carrying it — or its scroll
    /// offset — onto the next one shows a position in a different document.
    #[test]
    fn moving_drops_the_detail_and_its_offset() {
        let mut m = modal(3);
        m.detail = Some("the whole record".into());
        m.detail_scroll = 12;
        m.move_by(1);
        assert!(m.detail.is_none());
        assert_eq!(m.detail_scroll, 0);
    }

    #[test]
    fn switching_pane_starts_at_the_top() {
        let mut m = modal(5);
        m.move_by(4);
        m.set_pane(Pane::Rules, Vec::new());
        assert_eq!(m.selected, 0);
        assert_eq!(m.pane, Pane::Rules);
    }

    #[test]
    fn the_panes_cycle_both_ways() {
        assert_eq!(Pane::Reflections.next(), Pane::Rules);
        assert_eq!(Pane::Reflections.prev(), Pane::Proposals);
        assert_eq!(Pane::Proposals.next(), Pane::Reflections);
    }

    #[test]
    fn a_reflection_listing_carries_its_reason_and_its_marks() {
        let rows = rows_from_json(
            Pane::Reflections,
            r#"[{"id":"r1","title":"Use the other config.","domain":"behavior",
                 "blocked":"third-party content was in context","dropped":false,"edited":true}]"#,
        )
        .unwrap();
        assert_eq!(rows[0].id, "r1");
        assert!(rows[0].mine && !rows[0].spent);
        assert!(rows[0].note.as_deref().unwrap().contains("third-party"));
    }

    /// A rule no probe has reached is not a rule that passed, and the two must
    /// not render alike — the dash rule, one store over.
    #[test]
    fn a_never_validated_rule_says_so_rather_than_showing_zero() {
        let rows = rows_from_json(
            Pane::Rules,
            r#"[{"id":"a","title":"t","domain":"behavior","user":false,
                 "observations":0,"attributed_regressions":0,"retired":false},
                {"id":"b","title":"t","domain":"behavior","user":false,
                 "observations":9,"attributed_regressions":2,"retired":false},
                {"id":"c","title":"t","domain":"behavior","user":true,"retired":false}]"#,
        )
        .unwrap();
        assert!(rows[0].note.as_deref().unwrap().contains("never validated"));
        assert!(rows[1].note.as_deref().unwrap().contains("2 attributed"));
        assert!(rows[2].mine && rows[2].note.as_deref().unwrap().contains("yours"));
    }

    /// Against the shape `commands/proposals.rs::list` actually emits — `id`,
    /// `kind`, `title`, `detail` — never a `status` key. A fixture that
    /// hand-writes `status` is the scripted-provider trap this project's own
    /// name for it: the guarantee asserted against a belief about the
    /// producer rather than against what it produces, which is how the
    /// previous version of this test passed while every real row rendered
    /// dim and grey as though already decided.
    #[test]
    fn a_pending_proposal_is_never_spent_and_carries_its_domain() {
        let rows = rows_from_json(
            Pane::Proposals,
            r#"[{"id":"p1","kind":"behavior","title":"5 rule(s) from 10 reflection(s)","detail":"pending"},
                {"id":"p2","kind":"writing","title":"3 rule(s)","detail":"pending"}]"#,
        )
        .unwrap();
        assert!(
            !rows[0].spent,
            "list --json only ever returns pending items"
        );
        assert!(!rows[1].spent);
        assert_eq!(rows[0].tag, "behavior");
        assert_eq!(rows[0].note.as_deref(), Some("pending"));
    }

    #[test]
    fn it_draws_at_tiny_sizes() {
        // The `list_height` panic this project swept eight sites for: a
        // saturating subtraction to zero, then `clamp(min, max)` asserting.
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(20, 3)).unwrap();
        let mut m = modal(6);
        m.status = Some("dropped r0".into());
        term.draw(|f| m.draw(f)).unwrap();
        m.detail = Some("a very long record that has to wrap somewhere".repeat(4));
        term.draw(|f| m.draw(f)).unwrap();
        m.help = true;
        term.draw(|f| m.draw(f)).unwrap();
    }
}
