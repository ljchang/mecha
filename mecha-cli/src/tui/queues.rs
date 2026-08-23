//! The /review modal: everything waiting on a human, in one place.
//!
//! The `/tasks` pattern throughout — **every read and every mutation drives
//! `mecha review …` as a child process**, so there is one implementation per
//! verb and nothing this modal can do that the command line cannot.
//!
//! Three levels, and the middle one is the point:
//!
//! ```text
//!   queues ──Enter──▸ proposers ──Enter──▸ candidates
//!      │                                        a / r
//!      └─ outbox · front door · proposals ──▸ their own modals
//! ```
//!
//! The graph's merge queue is reviewed here; the other three rows hand off to
//! the sibling modal that already owns them. That asymmetry is deliberate:
//! duplicating `/outbox`'s send confirmations and taint warnings inside this
//! file would be a second implementation of the surface whose whole job is
//! making a person read before approving.
//!
//! **A candidate is never accepted from the model's side.** The graph's MCP
//! surface has `kg_pending` and `kg_verdict` and deliberately no `kg_accept`;
//! what runs here is the owner's `mecha-graph` binary, driven by a keystroke
//! from a person at a keyboard. See `commands::review` for the whole argument.
//!
//! Nothing rendered in a modal reaches a model, so a candidate's own words —
//! which came out of somebody's mail or Slack — stay on the human's side of
//! the boundary, exactly as `/tasks` keeps a task's.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

/// Which level the modal is showing.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Level {
    Queues,
    Proposers,
    Candidates,
    Items,
}

/// One store's backlog, as `mecha review queues --json` reports it.
pub struct QueueRow {
    pub name: String,
    /// `None` when the store could not be read. Rendered as a dash and never
    /// as zero — "nothing waiting" and "could not look" are opposite findings,
    /// and the whole reason this modal exists is that a queue grew unnoticed.
    pub depth: Option<usize>,
    pub detail: String,
    /// The `mecha …` verb that owns this queue, shown so the modal never
    /// becomes the only way to reach it.
    pub opens: String,
}

impl QueueRow {
    /// Whether this row is reviewed inside this modal or hands off.
    ///
    /// Keyed on the queue name the command emits, which is the same string on
    /// both sides of one process boundary — a second enum here would be a
    /// second list of queues to keep in step.
    pub fn is_graph(&self) -> bool {
        self.name == "graph candidates"
    }
}

/// One proposing mechanism.
pub struct ProposerRow {
    pub proposer: String,
    pub pending: usize,
    pub classes: usize,
    pub accepted: i64,
    pub rejected: i64,
    pub machine_rejected: i64,
    /// Wilson lower bound, `None` when no human has voted.
    pub accept_lb: Option<f64>,
}

impl ProposerRow {
    pub fn judged(&self) -> i64 {
        self.accepted + self.rejected
    }
    pub fn rate(&self) -> Option<f64> {
        match self.judged() {
            0 => None,
            n => Some(self.accepted as f64 / n as f64),
        }
    }
    /// How much the rate rests on, as a word. A bare percentage reads the
    /// same at n=2 and n=200.
    pub fn tier(&self) -> Tier {
        Tier::of(self.judged())
    }
    pub fn evidence(&self) -> &'static str {
        self.tier().as_str()
    }
}

/// How much of your own judgement a rate rests on.
///
/// The bucket is displayed on every row and is also selectable, because the
/// work that actually moves this queue is concentrated in one tier: 660
/// classes have no human verdict at all, and they sit scattered through a
/// list ordered by size, interleaved with the eighteen that are already
/// settled. Reading the label on every row to find them is how a backlog
/// stays a backlog.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Tier {
    Unjudged,
    Thin,
    Some,
    Solid,
}

impl Tier {
    /// The bucket a verdict count falls in. One definition, used by the
    /// label and by the filter — two would drift, and a filter that
    /// disagreed with the column beside it is worse than no filter.
    pub fn of(judged: i64) -> Tier {
        match judged {
            0 => Tier::Unjudged,
            1..=9 => Tier::Thin,
            10..=29 => Tier::Some,
            _ => Tier::Solid,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Unjudged => "unjudged",
            Tier::Thin => "thin",
            Tier::Some => "some",
            Tier::Solid => "solid",
        }
    }
    /// `t` cycles through the tiers and back to everything. Ordered
    /// least-evidence first, because that is the end of the list somebody
    /// opening this filter is looking for.
    pub fn next(current: Option<Tier>) -> Option<Tier> {
        match current {
            None => Option::Some(Tier::Unjudged),
            Option::Some(Tier::Unjudged) => Option::Some(Tier::Thin),
            Option::Some(Tier::Thin) => Option::Some(Tier::Some),
            Option::Some(Tier::Some) => Option::Some(Tier::Solid),
            Option::Some(Tier::Solid) => None,
        }
    }
}

/// One individual candidate, as `mecha review sample --json` reports it.
pub struct ItemRow {
    pub id: i64,
    pub statement: String,
    pub confidence: f64,
    /// The candidate's full payload, pretty-printed. The list shows one
    /// truncated line; this is where the rest of it lives — the same split
    /// `/tasks` makes, and for the same reason: a verdict on text you could
    /// not read is the approving-unread failure the outbox exists to
    /// prevent, one store over.
    pub payload: String,
    /// When the graph recorded the proposal.
    pub created_at: String,
}

/// One pending class, as `mecha review list --json` reports it. The graph
/// clusters by (proposer, predicate), so a row here is a class rather than a
/// single fact — which is the unit the queue is actually decidable in.
pub struct CandidateRow {
    pub proposer: String,
    pub predicate: String,
    pub pending: usize,
    pub accepted: i64,
    pub rejected: i64,
    pub samples: Vec<String>,
}

impl CandidateRow {
    pub fn judged(&self) -> i64 {
        self.accepted + self.rejected
    }
    pub fn tier(&self) -> Tier {
        Tier::of(self.judged())
    }
}

pub struct QueuesModal {
    pub level: Level,
    pub queues: Vec<QueueRow>,
    pub proposers: Vec<ProposerRow>,
    pub candidates: Vec<CandidateRow>,
    pub items: Vec<ItemRow>,
    pub selected: usize,
    /// The class the item level is drawn from.
    pub item_class: Option<(String, String)>,
    /// The seed that produced `items`, so the footer can name it — a sample
    /// nobody can redraw is a sample nobody can check.
    pub item_seed: Option<u64>,
    /// Full view of the selected item (`Enter` at the item level). j/k keep
    /// working and flip through items in place, so a sitting can be reviewed
    /// entirely from the detail — which is the reading a one-line truncation
    /// cannot give.
    pub item_detail: bool,
    /// How far the detail is scrolled. Reset on every move: an offset
    /// carried onto another item is a position in a different document —
    /// the `/tasks` detail_scroll lesson.
    pub detail_scroll: u16,
    /// Show only classes/mechanisms at this evidence tier. `None` is
    /// everything. Applied at render, not at load: the rows are already in
    /// hand, and a filter that re-ran the child process would make a display
    /// toggle cost a subprocess.
    pub tier: Option<Tier>,
    /// The mechanism the candidate list is narrowed to, if any.
    pub filter: Option<String>,
    pub status: Option<String>,
    pub help: bool,
}

impl QueuesModal {
    pub fn new(queues: Vec<QueueRow>) -> Self {
        Self {
            level: Level::Queues,
            queues,
            proposers: vec![],
            candidates: vec![],
            items: vec![],
            selected: 0,
            item_class: None,
            item_seed: None,
            item_detail: false,
            detail_scroll: 0,
            tier: None,
            filter: None,
            status: None,
            help: false,
        }
    }

    /// The mechanisms the tier filter admits.
    ///
    /// Every consumer — the row count, the cursor, the rendering — goes
    /// through this, so a filtered list cannot end up with a cursor pointing
    /// at a row nobody can see. That is the `/outbox` hidden-items bug in a
    /// list where the next keypress may be `r`.
    pub fn visible_proposers(&self) -> Vec<&ProposerRow> {
        self.proposers
            .iter()
            .filter(|p| self.tier.is_none_or(|t| p.tier() == t))
            .collect()
    }

    pub fn visible_candidates(&self) -> Vec<&CandidateRow> {
        self.candidates
            .iter()
            .filter(|c| self.tier.is_none_or(|t| c.tier() == t))
            .collect()
    }

    pub fn len(&self) -> usize {
        match self.level {
            Level::Queues => self.queues.len(),
            Level::Proposers => self.visible_proposers().len(),
            Level::Candidates => self.visible_candidates().len(),
            Level::Items => self.items.len(),
        }
    }

    /// Cycle the tier filter, and put the cursor back at the top.
    ///
    /// Resetting is the safe direction: the filtered list is a different
    /// list, so an index carried across names a different row to the next
    /// keypress — and at the class level the next keypress may verdict
    /// everything in it.
    pub fn cycle_tier(&mut self) {
        self.tier = Tier::next(self.tier);
        self.selected = 0;
    }

    /// Whether the current level has evidence to filter on at all. Items are
    /// individual candidates and carry no verdict history of their own.
    pub fn tier_applies(&self) -> bool {
        matches!(self.level, Level::Proposers | Level::Candidates)
    }

    pub fn move_sel(&mut self, delta: i32) {
        let len = self.len();
        if len == 0 {
            return;
        }
        let cur = self.selected.min(len - 1) as i32;
        self.selected = (cur + delta).clamp(0, len as i32 - 1) as usize;
    }

    pub fn selected_queue(&self) -> Option<&QueueRow> {
        self.queues.get(self.selected)
    }
    pub fn selected_proposer(&self) -> Option<&ProposerRow> {
        self.visible_proposers().get(self.selected).copied()
    }
    pub fn selected_candidate(&self) -> Option<&CandidateRow> {
        self.visible_candidates().get(self.selected).copied()
    }
    pub fn selected_item(&self) -> Option<&ItemRow> {
        self.items.get(self.selected)
    }

    fn list_scroll(&self, visible: u16) -> u16 {
        let visible = visible.max(1) as usize;
        (self.selected + 1).saturating_sub(visible) as u16
    }

    /// The filter, spelled for the title. A narrowed list that does not say
    /// so is a list that looks like the queue got smaller.
    fn tier_suffix(&self) -> String {
        match self.tier {
            Some(t) => format!(" · {} only", t.as_str()),
            None => String::new(),
        }
    }

    fn title(&self) -> String {
        match self.level {
            Level::Queues => {
                let total: usize = self.queues.iter().filter_map(|q| q.depth).sum();
                format!(" review — {total} waiting ")
            }
            Level::Proposers => {
                let shown: Vec<_> = self.visible_proposers();
                let total: usize = shown.iter().map(|p| p.pending).sum();
                format!(
                    " review · proposers — {total} pending in {}{} ",
                    shown.len(),
                    self.tier_suffix()
                )
            }
            Level::Candidates => {
                let total: usize = self.visible_candidates().iter().map(|c| c.pending).sum();
                let sfx = self.tier_suffix();
                match &self.filter {
                    Some(f) => format!(" review · {f} — {total} pending{sfx} "),
                    None => format!(" review · classes — {total} pending{sfx} "),
                }
            }
            Level::Items => {
                let cls = self
                    .item_class
                    .as_ref()
                    .map(|(p, pr)| format!("{p} · {pr}"))
                    .unwrap_or_else(|| "items".into());
                match self.item_seed {
                    Some(sd) => format!(
                        " {cls} — random sample of {} · seed {sd} ",
                        self.items.len()
                    ),
                    None => format!(" {cls} — {} item(s) ", self.items.len()),
                }
            }
        }
    }

    fn key_strip(&self) -> String {
        match self.level {
            Level::Queues => "j/k move · Enter open · ? help · Esc close".into(),
            Level::Proposers => {
                "j/k move · Enter classes · t evidence filter · Esc back · ? help".into()
            }
            Level::Candidates => {
                "j/k · Enter sample · a/r verdict WHOLE class · t filter · Esc back".into()
            }
            Level::Items => {
                "j/k · Enter full · a accept · r reject · b bind subject · A accept new · n resample".into()
            }
        }
    }

    pub fn draw(&self, frame: &mut Frame) {
        if self.help {
            self.draw_help(frame);
            return;
        }
        // Only while there is an item to show: a verdict can empty the
        // sample from inside the detail, and a blank box would strand the
        // keys — fall through to the list, which says what happened.
        if self.level == Level::Items && self.item_detail && self.selected_item().is_some() {
            self.draw_item_detail(frame);
            return;
        }
        let strip_text = format!("  {}", self.key_strip());
        let strip = Line::styled(strip_text.clone(), Style::new().fg(Color::Cyan));
        let body = match self.level {
            Level::Queues => self.queue_lines(),
            Level::Proposers => self.proposer_lines(),
            Level::Candidates => self.candidate_lines(),
            Level::Items => self.item_lines(),
        };

        let width = 122u16.min(frame.area().width);
        let strip_lines = strip_height(&strip_text, width.saturating_sub(2));
        // Status occupies a line when present, so it is reserved with the
        // strip rather than allowed to push the list past the box.
        let reserved = strip_lines + u16::from(self.status.is_some());
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
        let lines = strip_height(&strip_text, inner.width);
        frame.render_widget(
            Paragraph::new(strip).wrap(Wrap { trim: false }),
            Rect {
                height: lines.min(inner.height),
                ..inner
            },
        );
        let mut used = lines;
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
        frame.render_widget(
            Paragraph::new(body).scroll((self.list_scroll(list.height), 0)),
            list,
        );
    }

    fn queue_lines(&self) -> Vec<Line<'static>> {
        self.queues
            .iter()
            .enumerate()
            .map(|(i, q)| {
                let sel = i == self.selected;
                let marker = if sel { "›" } else { " " };
                let depth = match q.depth {
                    Some(n) => format!("{n:>6}"),
                    None => format!("{:>6}", "—"),
                };
                let here = if q.is_graph() { "review here" } else { "opens" };
                let text = format!(
                    "{marker} {depth}  {:<22} {:<11} {}",
                    q.name,
                    here,
                    truncate(&q.detail, 62)
                );
                style_row(text, sel, q.depth.is_none(), q.depth == Some(0))
            })
            .collect()
    }

    fn proposer_lines(&self) -> Vec<Line<'static>> {
        let visible = self.visible_proposers();
        if visible.is_empty() {
            return vec![Line::styled(
                format!(
                    "  no mechanism at tier `{}` — t cycles",
                    self.tier.map(Tier::as_str).unwrap_or("all")
                ),
                Style::new().fg(Color::DarkGray),
            )];
        }
        visible
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let sel = i == self.selected;
                let marker = if sel { "›" } else { " " };
                // A dash, never 0% — the distinction this whole surface turns on.
                let rate = match p.rate() {
                    Some(r) => format!("{:>4.0}% of {:<5}", r * 100.0, p.judged()),
                    None => format!("{:>4}  {:<7}", "—", "none"),
                };
                let text = format!(
                    "{marker} {:>6} in {:<4} {:<24} {rate} {:<9} {} auto-dropped",
                    p.pending,
                    p.classes,
                    truncate(&p.proposer, 24),
                    p.evidence(),
                    p.machine_rejected
                );
                let weak = p.accept_lb.is_some_and(|lb| lb < 0.25);
                style_row(text, sel, false, weak)
            })
            .collect()
    }

    fn candidate_lines(&self) -> Vec<Line<'static>> {
        let visible = self.visible_candidates();
        if visible.is_empty() {
            let msg = match self.tier {
                Some(t) => format!("  no class at tier `{}` here — t cycles", t.as_str()),
                None => "  nothing pending here".to_string(),
            };
            return vec![Line::styled(msg, Style::new().fg(Color::DarkGray))];
        }
        visible
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let sel = i == self.selected;
                let marker = if sel { "›" } else { " " };
                let hist = match c.judged() {
                    0 => "—".to_string(),
                    n => format!("{:.0}% of {n}", 100.0 * c.accepted as f64 / n as f64),
                };
                let sample = c.samples.first().map(String::as_str).unwrap_or("");
                let text = format!(
                    "{marker} {:>5}  {:<22} {:<12} {:<9} {}",
                    c.pending,
                    truncate(&c.predicate, 22),
                    hist,
                    c.tier().as_str(),
                    truncate(sample, 46)
                );
                style_row(text, sel, false, c.judged() == 0)
            })
            .collect()
    }

    fn item_lines(&self) -> Vec<Line<'static>> {
        if self.items.is_empty() {
            return vec![Line::styled(
                "  nothing left in this class",
                Style::new().fg(Color::DarkGray),
            )];
        }
        self.items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let sel = i == self.selected;
                let marker = if sel { "\u{203a}" } else { " " };
                let text = format!(
                    "{marker} #{:<7} {:.2}  {}",
                    it.id,
                    it.confidence,
                    truncate(&it.statement, 96)
                );
                style_row(text, sel, false, false)
            })
            .collect()
    }

    /// The whole candidate: full statement, then the payload the graph
    /// holds. What a verdict is actually about, readable before it is given.
    fn draw_item_detail(&self, frame: &mut Frame) {
        let Some(it) = self.selected_item() else {
            return;
        };
        let strip = "  j/k next · a accept · r reject · b bind subject · A accept new · Esc back";
        let mut body: Vec<Line> = vec![
            Line::styled(strip.to_string(), Style::new().fg(Color::Cyan)),
            Line::raw(""),
        ];
        // The statement first and wrapped — it is the thing being judged.
        for chunk in wrap_text(&it.statement, 96) {
            body.push(Line::styled(
                format!("  {chunk}"),
                Style::new().fg(Color::White).add_modifier(Modifier::BOLD),
            ));
        }
        body.push(Line::raw(""));
        let mut meta = format!("  #{} · confidence {:.2}", it.id, it.confidence);
        if !it.created_at.is_empty() {
            meta.push_str(&format!(" · proposed {}", it.created_at));
        }
        body.push(Line::styled(meta, Style::new().fg(Color::DarkGray)));
        body.push(Line::raw(""));
        body.push(Line::styled(
            "  ─ payload ─",
            Style::new().fg(Color::DarkGray),
        ));
        for l in it.payload.lines() {
            body.push(Line::styled(format!("  {l}"), Style::new().fg(Color::Gray)));
        }
        let width = 110u16.min(frame.area().width);
        let height = super::list_height(body.len() as u16, frame.area().height);
        let area = super::centered(frame.area(), width, height);
        frame.render_widget(Clear, area);
        let title = format!(
            " #{} — item {} of {} · {} ",
            it.id,
            self.selected + 1,
            self.items.len(),
            self.item_class
                .as_ref()
                .map(|(p, pr)| format!("{p} · {pr}"))
                .unwrap_or_default()
        );
        frame.render_widget(
            Paragraph::new(body)
                .wrap(Wrap { trim: false })
                .scroll((self.detail_scroll, 0))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::new().fg(Color::Cyan))
                        .title(title),
                ),
            area,
        );
    }

    fn draw_help(&self, frame: &mut Frame) {
        let body: Vec<Line> = HELP
            .lines()
            .map(|l| Line::styled(l.to_string(), Style::new().fg(Color::White)))
            .collect();
        let width = 100u16.min(frame.area().width);
        let height = super::list_height(body.len() as u16, frame.area().height);
        let area = super::centered(frame.area(), width, height);
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(body).wrap(Wrap { trim: false }).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(Color::Cyan))
                    .title(" review — keys "),
            ),
            area,
        );
    }
}

const HELP: &str = "
  Everything waiting on a human, in one place.

  QUEUES
    Enter    open — the graph queue is reviewed here; the others hand
             off to /outbox, /frontdoor and the proposals command, which
             own their own confirmations.
    A dash in the count means the store could not be read, which is not
    the same as nothing waiting.

  PROPOSERS  (the graph queue, by proposing mechanism)
    The rate is YOUR verdicts only. Rejections this pipeline made itself
    — duplicates, ephemerals — are shown separately as auto-dropped and
    never folded in, because a mechanism that mostly repeats itself is a
    different problem from one that is mostly wrong.
    'unjudged' means no human has ever voted on it. It is not a zero.

  t  (proposers and classes)
    Cycle the evidence filter: all → unjudged → thin → some → solid.
    The tier is how many verdicts of YOUR OWN the rate rests on, so
    `unjudged` is the set with no basis at all — 660 classes here, and
    the only ones sampling actually buys anything on. The cursor returns
    to the top on every change, because a filtered list is a different
    list and the next keypress may verdict a whole class.

  CLASSES
    Enter    a RANDOM sample of this class, to review one at a time
    a        accept the whole class
    r        reject the whole class
    Verdicts on a whole class are for one you have already decided about.
    To learn whether a class is any good, sample it.

  ITEMS  (a random sample, seeded so it can be redrawn)
    Enter    the full item — whole statement and payload; j/k flips
             through items without leaving it
    a        accept this one (returns to the list, row removed)
    r        reject this one (same)
    b        an accept failed on `cannot resolve subject`? bind the
             subject to the graph's closest entity — the old spelling
             becomes an alias, so the fix outlives this item — then a
    A        accept creating the subject as a NEW topic node, for a
             subject that is genuinely new rather than misspelled
    n        draw a new sample
    The draw is random because the queue is ordered, and every order it
    could have is correlated with something. Judging the first dozen and
    calling the result the class's accept rate measures the ordering.
    The seed is in the title: quote it and the sample can be checked.

    Both levels run mecha-graph as a child process. Nothing a model can
    call accepts a candidate — that is the point of the split.

  Esc backs out one level at a time.
";

/// One row's styling. Selection wins, then unreadable, then dimmed.
fn style_row(text: String, selected: bool, unreadable: bool, dim: bool) -> Line<'static> {
    if selected {
        Line::styled(text, Style::new().fg(Color::Black).bg(Color::Cyan))
    } else if unreadable {
        Line::styled(text, Style::new().fg(Color::Red))
    } else if dim {
        Line::styled(text, Style::new().fg(Color::DarkGray))
    } else {
        Line::styled(text, Style::new().fg(Color::White))
    }
}

/// Greedy word wrap. `Paragraph::wrap` exists, but the statement needs its
/// own lines so the styling (bold) survives — a single styled Line wraps as
/// one span and keeps its style, so this is belt over braces only for the
/// indent staying even on continuation lines.
fn wrap_text(s: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn truncate(s: &str, n: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= n {
        return s;
    }
    s.chars()
        .take(n.saturating_sub(1))
        .chain(std::iter::once('…'))
        .collect()
}

fn strip_height(strip: &str, width: u16) -> u16 {
    let width = width.max(1) as usize;
    (strip.chars().count().div_ceil(width) as u16).max(1)
}

// ─── JSON in ─────────────────────────────────────────────────────────────────

pub fn queues_from_json(text: &str) -> anyhow::Result<Vec<QueueRow>> {
    let v: serde_json::Value = serde_json::from_str(text)?;
    Ok(v.as_array()
        .map(|rows| {
            rows.iter()
                .map(|r| QueueRow {
                    name: r["queue"].as_str().unwrap_or("?").to_string(),
                    // `null` is unreadable, and must not become 0.
                    depth: r["depth"].as_u64().map(|n| n as usize),
                    detail: r["detail"].as_str().unwrap_or("").to_string(),
                    opens: r["opens"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default())
}

pub fn proposers_from_json(text: &str) -> anyhow::Result<Vec<ProposerRow>> {
    let v: serde_json::Value = serde_json::from_str(text)?;
    Ok(v.as_array()
        .map(|rows| {
            rows.iter()
                .map(|r| ProposerRow {
                    proposer: r["proposer"].as_str().unwrap_or("?").to_string(),
                    pending: r["pending"].as_u64().unwrap_or(0) as usize,
                    classes: r["classes"].as_u64().unwrap_or(0) as usize,
                    accepted: r["accepted_hist"].as_i64().unwrap_or(0),
                    rejected: r["rejected_hist"].as_i64().unwrap_or(0),
                    machine_rejected: r["machine_rejected"].as_i64().unwrap_or(0),
                    accept_lb: r["accept_lb"].as_f64(),
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Individual candidates, as `mecha-graph review --json` serialises a
/// `FactCandidate`. The statement lives under `payload`, with `what` as the
/// commitment-shaped alternative — the same two keys the graph's own views
/// look under, so a commitment does not render blank here alone.
pub fn items_from_json(text: &str) -> anyhow::Result<Vec<ItemRow>> {
    let v: serde_json::Value = serde_json::from_str(text)?;
    Ok(v.as_array()
        .map(|rows| {
            rows.iter()
                .map(|r| ItemRow {
                    id: r["id"].as_i64().unwrap_or(0),
                    statement: r["payload"]["statement"]
                        .as_str()
                        .or_else(|| r["payload"]["what"].as_str())
                        .unwrap_or("(no statement)")
                        .to_string(),
                    confidence: r["confidence"].as_f64().unwrap_or(0.0),
                    payload: serde_json::to_string_pretty(&r["payload"])
                        .unwrap_or_else(|_| "{}".into()),
                    created_at: r["created_at"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default())
}

pub fn candidates_from_json(text: &str) -> anyhow::Result<Vec<CandidateRow>> {
    let v: serde_json::Value = serde_json::from_str(text)?;
    Ok(v.as_array()
        .map(|rows| {
            rows.iter()
                .map(|r| CandidateRow {
                    proposer: r["proposed_by"].as_str().unwrap_or("?").to_string(),
                    predicate: r["predicate"].as_str().unwrap_or("?").to_string(),
                    pending: r["pending"].as_u64().unwrap_or(0) as usize,
                    accepted: r["accepted_hist"].as_i64().unwrap_or(0),
                    rejected: r["rejected_hist"].as_i64().unwrap_or(0),
                    samples: r["samples"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|s| s.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An unreadable store must not render as an empty one.
    ///
    /// `depth: null` is what the command emits when it could not open a
    /// store, and the whole reason this modal exists is that a queue grew to
    /// 6,434 without anyone noticing — a reader that reported its own failure
    /// as "nothing waiting" would reproduce that exactly.
    #[test]
    fn an_unreadable_queue_is_none_and_not_zero() {
        let rows = queues_from_json(
            r#"[{"queue":"graph candidates","depth":null,"detail":"binary missing","opens":"x"},
                {"queue":"outbox drafts","depth":0,"detail":"","opens":"y"}]"#,
        )
        .unwrap();
        assert_eq!(rows[0].depth, None, "null stays unknown");
        assert_eq!(rows[1].depth, Some(0), "a real zero is a real zero");
        assert!(rows[0].is_graph());
        assert!(!rows[1].is_graph());
    }

    /// A proposer nobody has judged has no rate, and no evidence word that
    /// could be mistaken for one.
    #[test]
    fn an_unjudged_proposer_has_no_rate() {
        let rows = proposers_from_json(
            r#"[{"proposer":"bee:suggested","pending":1084,"classes":1,
                 "accepted_hist":0,"rejected_hist":0,"machine_rejected":16,"accept_lb":null},
                {"proposer":"llm","pending":4841,"classes":726,
                 "accepted_hist":1175,"rejected_hist":809,"machine_rejected":1167,
                 "accept_lb":0.5717}]"#,
        )
        .unwrap();
        assert_eq!(rows[0].rate(), None);
        assert_eq!(rows[0].evidence(), "unjudged");
        assert_eq!(rows[0].machine_rejected, 16);
        assert_eq!(rows[1].evidence(), "solid");
        assert!((rows[1].rate().unwrap() - 0.5923).abs() < 0.001);
    }

    /// Moving the cursor never leaves the list, and never panics on an empty
    /// one — the modal opens on whatever the stores happen to hold.
    #[test]
    fn selection_stays_inside_the_list() {
        let mut m = QueuesModal::new(vec![]);
        m.move_sel(1);
        assert_eq!(m.selected, 0, "an empty list has nowhere to go");
        m.queues = queues_from_json(
            r#"[{"queue":"a","depth":1,"detail":"","opens":""},
                {"queue":"b","depth":2,"detail":"","opens":""}]"#,
        )
        .unwrap();
        m.move_sel(5);
        assert_eq!(m.selected, 1, "clamped to the last row");
        m.move_sel(-5);
        assert_eq!(m.selected, 0, "and to the first");
    }

    /// A commitment-shaped candidate has `what`, not `statement`, and must
    /// not render blank — it is the one payload shape that differs.
    #[test]
    fn an_item_renders_from_either_payload_shape() {
        let rows = items_from_json(
            r#"[{"id":12,"confidence":0.9,"payload":{"statement":"A works at B","predicate":"works_at"}},
                {"id":13,"confidence":0.5,"payload":{"what":"send the draft","kind":"commitment"}},
                {"id":14,"confidence":0.1,"payload":{}}]"#,
        )
        .unwrap();
        assert_eq!(rows[0].statement, "A works at B");
        assert_eq!(
            rows[1].statement, "send the draft",
            "commitments use `what`"
        );
        assert_eq!(rows[2].statement, "(no statement)", "and never blank");
        assert_eq!(rows[0].id, 12);
        assert!(
            rows[0].payload.contains("works_at"),
            "the detail view gets the whole payload: {}",
            rows[0].payload
        );
    }

    /// The detail shows the full statement the list truncated.
    ///
    /// The list clips at ~96 characters, and a verdict on text you could not
    /// read is the approving-unread failure the outbox exists to prevent —
    /// this is the screenshot bug: an item whose statement ended in "and pe…"
    /// with no way to see the rest.
    #[test]
    fn the_detail_carries_what_the_list_truncates() {
        let long = "Possible duplicate: person node person-5ef7b325 (Grace Choi) and person node person-9a1b2c3d (Grace H. Choi) share an email identifier and forty-one overlapping calendar events".to_string();
        let rows = items_from_json(&format!(
            r#"[{{"id":1737,"confidence":0.8,"payload":{{"statement":"{long}"}}}}]"#
        ))
        .unwrap();
        assert_eq!(
            rows[0].statement, long,
            "nothing lost between JSON and detail"
        );
        let wrapped = wrap_text(&rows[0].statement, 96);
        assert!(wrapped.len() > 1, "and it wraps rather than clips");
        assert_eq!(
            wrapped.join(" "),
            long,
            "wrapping reflows; it never drops a word"
        );
    }

    fn proposer(name: &str, pending: usize, a: i64, r: i64) -> ProposerRow {
        ProposerRow {
            proposer: name.into(),
            pending,
            classes: 1,
            accepted: a,
            rejected: r,
            machine_rejected: 0,
            accept_lb: if a + r == 0 { None } else { Some(0.5) },
        }
    }

    /// The filter selects on the same buckets the column displays.
    ///
    /// One definition (`Tier::of`) behind both. Two would drift, and a filter
    /// that disagreed with the word printed beside it is worse than no
    /// filter — you would reject a class believing it was in a tier it was
    /// not.
    #[test]
    fn the_tier_filter_and_the_tier_label_agree() {
        let mut m = QueuesModal::new(vec![]);
        m.level = Level::Proposers;
        m.proposers = vec![
            proposer("bee:suggested", 1084, 0, 0),   // unjudged
            proposer("rule:x", 29, 3, 2),            // thin (5)
            proposer("llm:commitment", 421, 10, 14), // some (24)
            proposer("llm", 4841, 1175, 809),        // solid
        ];
        assert_eq!(m.len(), 4, "no filter shows everything");

        for (tier, expect) in [
            (Tier::Unjudged, "bee:suggested"),
            (Tier::Thin, "rule:x"),
            (Tier::Some, "llm:commitment"),
            (Tier::Solid, "llm"),
        ] {
            m.tier = Some(tier);
            m.selected = 0;
            let vis = m.visible_proposers();
            assert_eq!(vis.len(), 1, "exactly one at {tier:?}");
            assert_eq!(vis[0].proposer, expect);
            assert_eq!(
                vis[0].evidence(),
                tier.as_str(),
                "the printed label is the bucket the filter selected on"
            );
        }
    }

    /// `t` cycles through every tier and back to everything, and the cursor
    /// never survives the change.
    ///
    /// A filtered list is a different list, so an index carried across names
    /// a different row — and at the class level the next keypress verdicts
    /// everything in that row.
    #[test]
    fn cycling_the_tier_resets_the_cursor_and_returns_to_all() {
        let mut m = QueuesModal::new(vec![]);
        m.level = Level::Proposers;
        m.proposers = vec![proposer("a", 1, 0, 0), proposer("b", 1, 0, 0)];
        m.selected = 1;
        let mut seen = vec![];
        for _ in 0..5 {
            m.cycle_tier();
            assert_eq!(m.selected, 0, "cursor home on every change");
            seen.push(m.tier);
        }
        assert_eq!(
            seen,
            vec![
                Some(Tier::Unjudged),
                Some(Tier::Thin),
                Some(Tier::Some),
                Some(Tier::Solid),
                None
            ],
            "least evidence first, then back to everything"
        );
    }

    /// Selection reads the filtered list, never the raw one.
    ///
    /// If `selected_proposer` indexed `self.proposers` while the rows drawn
    /// came from the filtered view, a keystroke would act on a row that is
    /// not on screen.
    #[test]
    fn selection_follows_the_filter_not_the_raw_list() {
        let mut m = QueuesModal::new(vec![]);
        m.level = Level::Proposers;
        m.proposers = vec![
            proposer("solid-one", 10, 40, 10),
            proposer("unjudged-one", 20, 0, 0),
        ];
        m.tier = Some(Tier::Unjudged);
        m.selected = 0;
        assert_eq!(
            m.selected_proposer().map(|p| p.proposer.as_str()),
            Some("unjudged-one"),
            "row 0 of the FILTERED list, not of the raw one"
        );
        m.move_sel(1);
        assert_eq!(m.selected, 0, "and it cannot move past the filtered end");
    }

    /// The box must draw at sizes where the naive clamp panics.
    ///
    /// `rows.clamp(1, height - 4)` asserts `min <= max` the moment the
    /// terminal is four rows or fewer, which took whole sessions down from
    /// seven other modals. The assertion here IS the draw.
    #[test]
    fn it_draws_at_tiny_sizes() {
        let mut m = QueuesModal::new(
            queues_from_json(
                r#"[{"queue":"graph candidates","depth":6434,"detail":"d","opens":"o"}]"#,
            )
            .unwrap(),
        );
        m.status = Some("accepted 12".into());
        for h in 1..=8u16 {
            for w in [8u16, 40, 130] {
                let backend = ratatui::backend::TestBackend::new(w, h);
                let mut term = Terminal::new(backend).unwrap();
                term.draw(|f| m.draw(f)).unwrap();
            }
        }
        for level in [Level::Candidates, Level::Items] {
            m.level = level;
            for h in 1..=6u16 {
                let backend = ratatui::backend::TestBackend::new(30, h);
                let mut term = Terminal::new(backend).unwrap();
                term.draw(|f| m.draw(f)).unwrap();
            }
        }
        // The item detail, including at sizes where the naive clamp panics,
        // and the emptied-sample fall-through.
        m.level = Level::Items;
        m.items = items_from_json(
            r#"[{"id":9,"confidence":0.8,"created_at":"2026-08-22 04:54:25",
                 "payload":{"statement":"A very long statement that will need wrapping across several lines to be read in full","predicate":"related_to","subject":"A","object":"B"}}]"#,
        )
        .unwrap();
        m.item_detail = true;
        for h in 1..=8u16 {
            let backend = ratatui::backend::TestBackend::new(40, h);
            let mut term = Terminal::new(backend).unwrap();
            term.draw(|f| m.draw(f)).unwrap();
        }
        m.items.clear();
        let backend = ratatui::backend::TestBackend::new(40, 8);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| m.draw(f)).unwrap(); // detail open, nothing left — must not blank
    }
}
