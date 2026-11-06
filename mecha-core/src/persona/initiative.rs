//! Who brings her people into a persona's chat, and what the owner does
//! with them when she does, read back from the chats' transcripts
//! (PERSONA-LIFE-DESIGN.md §14).
//!
//! **Why it exists.** Step 3 of persona lives makes things happen in her
//! life, and §14 asks for a reading before it is built: does a world make
//! her better company, or a companion who brings up her own life when
//! nobody asked (ComPeer's complaint)? Initiative alone cannot tell them
//! apart. It rises by construction once her people are in her prompt. So
//! it is read only beside **uptake**: what the owner does with someone she
//! brought up. Rising initiative with falling uptake is that failure,
//! measured.
//!
//! **Offline, like [`super::echo`].** Nothing is computed while a chat
//! runs and nothing new is stored. A reading is taken from the transcripts
//! when asked, and a better rule later recomputes the whole history.
//!
//! **What counts, in this first build.** Only her people, by the names and
//! aliases the owner wrote in her network (`persona::network`), read the
//! same over every chat, so a chat from before her people existed and one
//! from after are measured by one instrument. Events wait for step 3,
//! when they get names a transcript can be searched for. A name is matched
//! as whole words, and a capitalised name must be capitalised, since "will"
//! is not Will. The one exception is an owner turn typed with no capital
//! at all, as a phone often is ("is pell ok"), where case is ignored
//! (review of #808). A name that is also a common word still over-counts
//! where a sentence starts with it.
//!
//! - **An introduction** is a completed reply naming someone neither side
//!   has named earlier in that chat. Harness notes are not read, only what
//!   the owner typed and what she said, so her people riding in her prompt
//!   do not count as either side naming them.
//! - **Taken up** is the owner naming them in the next turn they speak
//!   in, and **asked** the subset where that turn holds a question mark.
//! - **Came back** is the owner naming them in another chat, later,
//!   before she does. Later in the same chat is not a return, however long
//!   the gap: a chat resumed after days reads on as the one chat it is.
//! - **Not taken up** is her bringing them up again first, or the owner
//!   speaking after the introduction without ever naming them: in the
//!   next turn, or, for a chat's last introduction, in any later chat.
//!   A chat usually ends on her turn, so that last case is the common one,
//!   and leaving it out would flatter the rate (review of #808).
//! - **Unanswered** is an introduction the owner has not spoken after at
//!   all: the newest chat's last turn. Its outcome is unknown, so it is
//!   kept out of the rate, never counted as a no.
//!
//! **Order is as near to when it was said as the record allows.** "Later"
//! is read in time, not by chat. A chat is read as stretches, split
//! wherever the transcript records a config with its clock, and every
//! chat's stretches are read in one order by that clock. Ordering whole
//! chats by when they began would put a long-lived chat's newest turns
//! before chats that came after them, and each misplaced turn errs toward
//! uptake (review of #808). The limit: no record carries a clock per run,
//! and the persona door writes a config only when a chat is first taken up
//! by a `serve` process or the router rebinds, not on every resume. So a
//! chat held open in one process across days is read at its start, and a
//! cross-chat return during those days can read as the chat's own next
//! turn, or the other way round. A split decides only that order: within a
//! chat, an introduction waits for the owner's next turn wherever it
//! falls, so where a chat happens to split changes no outcome.
//!
//! **Arms, from what she was shown, not from her settings.** Each turn is
//! filed by whether the transcript shows her days (a today-block among the
//! run's notes), the teller (`today::EVENTS_HEADING` in that block), and her
//! people (their heading in the recorded system prompt). Settings change,
//! and a chat keeps the version it began with, so the record is the only
//! honest source.
//!
//! **Counts, never words.** A reading holds numbers only. A name is
//! looked for, never kept, so a reading can be pasted anywhere.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use chrono::{DateTime, Utc};

use crate::message::{Message, Role};
use crate::session::{Record, Session};

use super::network::Network;

/// One of her people as a transcript is searched for them: every name the
/// owner wrote, each split into words.
#[derive(Debug, Clone, PartialEq)]
pub struct Subject {
    names: Vec<Vec<String>>,
}

impl Subject {
    fn new<'a>(names: impl IntoIterator<Item = &'a str>) -> Subject {
        Subject {
            names: names
                .into_iter()
                .map(words)
                .filter(|w| !w.is_empty())
                .collect(),
        }
    }
}

/// Everyone in `n`, as searched for.
pub fn subjects(n: &Network) -> Vec<Subject> {
    n.people
        .iter()
        .map(|p| {
            Subject::new(
                std::iter::once(p.name.as_str()).chain(p.aliases.iter().map(String::as_str)),
            )
        })
        .filter(|s| !s.names.is_empty())
        .collect()
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

fn capitalised(w: &str) -> bool {
    w.chars().next().is_some_and(char::is_uppercase)
}

/// Whether `text` names `s`. When `cased`, a word the owner capitalised
/// must be capitalised; otherwise case is ignored.
fn names(text: &[String], s: &Subject, cased: bool) -> bool {
    let same = |said: &str, name: &str| {
        said.to_lowercase() == name.to_lowercase()
            && !(cased && capitalised(name) && !capitalised(said))
    };
    s.names.iter().any(|name| {
        text.windows(name.len())
            .any(|w| w.iter().zip(name).all(|(said, n)| same(said, n)))
    })
}

/// What she was shown on one turn.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
pub struct Arm {
    /// A today-block among the run's notes.
    #[serde(rename = "shown_days")]
    pub days: bool,
    /// Her people in the recorded system prompt.
    #[serde(rename = "shown_people")]
    pub people: bool,
    /// The teller lines in that today-block.
    #[serde(default, rename = "shown_events")]
    pub events: bool,
}

/// One run of a chat, as this reading needs it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Turn {
    /// What the owner typed in the run, harness notes left out.
    pub owner: String,
    /// Her reply, and whether the run completed. A stopped run's partial
    /// reply was seen, so it counts as naming, but is not scored.
    pub reply: Option<String>,
    pub completed: bool,
    pub arm: Arm,
}

/// What the owner typed in `m`: [`crate::agent::owner_text`], less a
/// compaction's summary and carried blocks, which it does not exclude
/// ([`crate::title::is_derived`] does).
fn owner_words(m: &Message) -> String {
    if m.harness {
        return String::new();
    }
    m.content
        .iter()
        .filter_map(|b| match b {
            crate::message::Block::Text { text } if !crate::title::is_derived(text) => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect()
}

/// Whether `m` is a compaction's rebuilt head: a user message carrying the
/// summary or the carried block.
fn rebuilt(m: &Message) -> bool {
    m.content.iter().any(|b| {
        matches!(b, crate::message::Block::Text { text }
            if crate::title::is_derived(text) && !crate::agent::is_harness_voice(text))
    })
}

/// Add `more` to what she said in the run.
fn join(reply: &mut Option<String>, more: &str) {
    match reply {
        Some(r) => {
            r.push('\n');
            r.push_str(more);
        }
        None => *reply = Some(more.to_string()),
    }
}

/// The text of an assistant message with something to say.
fn reply_text(m: &Message) -> Option<String> {
    let text = m.text();
    (m.role == Role::Assistant && !text.trim().is_empty()).then_some(text)
}

/// One stretch of a chat: from its start, or a resume, to the next.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Stretch {
    /// The recorded config's clock; `None` on a record from before it.
    pub at: Option<DateTime<Utc>>,
    pub turns: Vec<Turn>,
}

/// One chat's runs, in order, from its session file, in stretches split
/// where a config is recorded (the chat's start, and each resume).
///
/// Walks the records as [`super::echo::session_echoes`] does: a run's
/// notes and messages (appended, or a `Rewrite` holding the whole history
/// when it compacted), in whatever order the door wrote them, then its
/// `Outcome`, where the run is closed. The owner's text is every
/// owner message appended in the run. After a rewrite it is the last
/// message the owner typed in the history, which is the run's own,
/// since what came before it was summarised away. The arm's people half
/// follows the latest recorded config, its days half the run's notes.
pub fn session_turns(path: &Path) -> Result<Vec<Stretch>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut out: Vec<Stretch> = Vec::new();
    let mut now = Stretch::default();
    let mut people = false;
    let mut run = Turn::default();
    // The history so far, as its length and last message: appended
    // messages move it, a rewrite sets it. And the same as it stood at the
    // last outcome, which a failed turn's rollback returns it to.
    let mut seen = (0usize, None::<String>);
    let mut flushed = (0usize, None::<String>);
    let key = |m: Option<&Message>| m.and_then(|m| serde_json::to_string(m).ok());
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str(line) {
            Ok(Record::Config(c)) => {
                if !now.turns.is_empty() {
                    out.push(std::mem::take(&mut now));
                }
                now.at = c.clock.or(now.at);
                people = c
                    .system_prompt
                    .as_deref()
                    .is_some_and(|s| s.contains(super::agent::PEOPLE_HEADING));
            }
            Ok(Record::Message(m)) => {
                seen = (seen.0 + 1, key(Some(&m)));
                if m.role == Role::User {
                    let said = owner_words(&m);
                    if !said.trim().is_empty() {
                        if !run.owner.is_empty() {
                            run.owner.push('\n');
                        }
                        run.owner.push_str(&said);
                    }
                } else if let Some(reply) = reply_text(&m) {
                    // Every text she said in the run, not only the last: a
                    // run that called a tool spoke before it too, and the
                    // owner saw both (review of #808).
                    join(&mut run.reply, &reply);
                }
            }
            Ok(Record::Rewrite { messages }) => {
                seen = (messages.len(), key(messages.last()));
                // A failed turn rolled back: the history returned to exactly
                // what it was at the last outcome, and no outcome follows.
                // It holds the turns before, already read; reading it as this
                // run would splice the last turn back in and drop what the
                // owner typed in the failed one, which stays and joins the
                // next turn, the one that answered it (review of #808). A
                // compaction is never that: it ends on this run's newest
                // message.
                if seen == flushed {
                    continue;
                }
                // The run's own part: everything after the last message the
                // owner typed.
                let from = messages
                    .iter()
                    .rposition(|m| m.role == Role::User && !owner_words(m).trim().is_empty())
                    .map_or(0, |i| i + 1);
                if from > 0 {
                    let head = &messages[from - 1];
                    // A compaction's rebuilt head: the owner's first words
                    // with a model-written summary joined on. Neither is
                    // this run's, which the summary swallowed, so the
                    // owner's text for it is unknown, never the summary
                    // (which names her people) read as theirs (review of
                    // #808).
                    run.owner = if rebuilt(head) {
                        String::new()
                    } else {
                        owner_words(head)
                    };
                }
                run.reply = None;
                for reply in messages[from..].iter().filter_map(reply_text) {
                    join(&mut run.reply, &reply);
                }
            }
            Ok(Record::Notes { notes }) => {
                if notes.iter().any(|n| super::today::stem_of(n).is_some()) {
                    run.arm.days = true;
                }
                if notes.iter().any(|n| {
                    super::today::stem_of(n).is_some() && n.contains(super::today::EVENTS_HEADING)
                }) {
                    run.arm.events = true;
                }
            }
            Ok(Record::Outcome(stats)) => {
                flushed = seen.clone();
                let mut done = std::mem::take(&mut run);
                done.completed = stats.stop_cause == Some(crate::agent::StopCause::Completed);
                done.arm.people = people;
                now.turns.push(done);
            }
            _ => {}
        }
    }
    if !now.turns.is_empty() {
        out.push(now);
    }
    Ok(out)
}

/// What one arm's turns say.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct Reading {
    /// Completed replies read.
    pub replies: usize,
    /// Of them, replies that introduced at least one of her people.
    pub introducing: usize,
    /// People introduced, counted once per reply that introduced them.
    pub introductions: usize,
    /// The owner named them in the next turn.
    pub taken_up: usize,
    /// Of those, a next turn with a question in it.
    pub asked: usize,
    /// The owner named them in a later chat before she did.
    pub came_back: usize,
    /// Neither, or she brought them up again first.
    pub not_taken_up: usize,
    /// In a chat's last turn with no return yet: no outcome, not a no.
    pub unanswered: usize,
}

impl Reading {
    /// The share of her replies that introduced someone; `None` over no
    /// replies.
    pub fn initiative(&self) -> Option<f64> {
        (self.replies > 0).then(|| self.introducing as f64 / self.replies as f64)
    }

    /// Introductions with an outcome: taken up, either way, or not.
    pub fn decided(&self) -> usize {
        self.taken_up + self.came_back + self.not_taken_up
    }

    /// Of the introductions with an outcome, the share the owner took up,
    /// in the next turn or a later chat; `None` when none has an outcome.
    pub fn uptake(&self) -> Option<f64> {
        let decided = self.decided();
        (decided > 0).then(|| (self.taken_up + self.came_back) as f64 / decided as f64)
    }
}

/// An introduction waiting on its outcome.
struct Waiting {
    arm: Arm,
    /// The chat it was made in: a return or a repeat is another chat's.
    chat: usize,
    /// The stretch it was made in, for whether the owner spoke after it.
    stretch: usize,
    /// Still waiting on the owner's next turn in its chat.
    next: bool,
    /// The owner had that next turn and did not name them.
    passed: bool,
}

/// How a waiting introduction ended.
enum Settled {
    /// The owner brought them up first in another chat.
    Back,
    /// She brought them up again first, in another chat.
    Repeated,
    /// The reading ran out: a no if the owner spoke after it, unknown if
    /// not.
    End,
}

fn settle(by: &mut HashMap<Arm, Reading>, w: Waiting, how: Settled) {
    let r = by.entry(w.arm).or_default();
    match how {
        Settled::Back => r.came_back += 1,
        Settled::Repeated => r.not_taken_up += 1,
        Settled::End if w.passed => r.not_taken_up += 1,
        Settled::End => r.unanswered += 1,
    }
}

/// Settle what another chat left waiting on `s`.
fn settle_elsewhere(
    by: &mut HashMap<Arm, Reading>,
    waiting: &mut HashMap<usize, Vec<Waiting>>,
    s: usize,
    chat: usize,
    how: fn() -> Settled,
) {
    let Some(ws) = waiting.get_mut(&s) else {
        return;
    };
    let (done, keep): (Vec<_>, Vec<_>) = ws.drain(..).partition(|w| w.chat != chat);
    *ws = keep;
    for w in done {
        settle(by, w, how());
    }
}

/// Every arm's reading over `stretches`, each `(chat, turns)`, in the
/// order they were said, searching for `who`. Arms with no reply are left
/// out.
pub fn score(stretches: &[(usize, Vec<Turn>)], who: &[Subject]) -> Vec<(Arm, Reading)> {
    let mut by: HashMap<Arm, Reading> = HashMap::new();
    // Per subject, introductions not yet settled.
    let mut waiting: HashMap<usize, Vec<Waiting>> = HashMap::new();
    // Named by either side earlier in each chat, across its stretches.
    let mut named: HashMap<usize, HashSet<usize>> = HashMap::new();
    // The last stretch the owner spoke in.
    let mut spoke: Option<usize> = None;
    for (stretch, (chat, turns)) in stretches.iter().enumerate() {
        let chat = *chat;
        let named = named.entry(chat).or_default();
        for turn in turns {
            let owner = words(&turn.owner);
            if !owner.is_empty() {
                spoke = Some(stretch);
                let asked = turn.owner.contains('?');
                let cased = turn.owner.chars().any(char::is_uppercase);
                let said: Vec<bool> = who.iter().map(|w| names(&owner, w, cased)).collect();
                // This chat's introductions waiting on this, its next turn.
                for (s, ws) in waiting.iter_mut() {
                    ws.retain_mut(|w| {
                        if w.chat != chat || !w.next {
                            return true;
                        }
                        if said[*s] {
                            let r = by.entry(w.arm).or_default();
                            r.taken_up += 1;
                            if asked {
                                r.asked += 1;
                            }
                            return false;
                        }
                        w.next = false;
                        w.passed = true;
                        true
                    });
                }
                for s in (0..who.len()).filter(|s| said[*s]) {
                    named.insert(s);
                    // The return another chat's introduction waited for.
                    settle_elsewhere(&mut by, &mut waiting, s, chat, || Settled::Back);
                }
            }
            let Some(reply) = &turn.reply else {
                continue;
            };
            let said = words(reply);
            let r = by.entry(turn.arm).or_default();
            if turn.completed {
                r.replies += 1;
            }
            let mut introduced = false;
            for (s, subject) in who.iter().enumerate() {
                if !names(&said, subject, true) {
                    continue;
                }
                // Her bringing them up again first, in another chat,
                // settles what that chat left waiting.
                settle_elsewhere(&mut by, &mut waiting, s, chat, || Settled::Repeated);
                if !named.insert(s) || !turn.completed {
                    continue;
                }
                introduced = true;
                by.entry(turn.arm).or_default().introductions += 1;
                waiting.entry(s).or_default().push(Waiting {
                    arm: turn.arm,
                    chat,
                    stretch,
                    next: true,
                    passed: false,
                });
            }
            if introduced {
                by.entry(turn.arm).or_default().introducing += 1;
            }
        }
    }
    // What is still waiting: a no if the owner spoke after it anywhere,
    // unknown if never.
    for mut w in waiting.into_values().flatten() {
        w.passed |= spoke.is_some_and(|c| c > w.stretch);
        settle(&mut by, w, Settled::End);
    }
    let mut out: Vec<(Arm, Reading)> = by.into_iter().filter(|(_, r)| r.replies > 0).collect();
    out.sort_by_key(|(a, _)| *a);
    out
}

/// What [`stretches`] read.
#[derive(Debug, Default)]
pub struct Said {
    /// `(chat, turns)`, in the order they were said.
    pub stretches: Vec<(usize, Vec<Turn>)>,
    /// Chats read.
    pub chats: usize,
    /// Session files that could not be read.
    pub unreadable: usize,
    /// Test or withdrawn sessions left out, counted
    /// ([`super::echo::is_owner_chat`]).
    pub left_out: usize,
}

/// Every stretch of every chat in `sessions` last written at or after
/// `since`, in the order they were said: by the stretch's clock, or its
/// chat's start where none was recorded.
pub fn stretches(sessions: &Path, since: std::time::SystemTime) -> Result<Said> {
    let (listed, mut unreadable) = Session::list_counting(sessions)?;
    let mut all: Vec<(DateTime<Utc>, usize, Vec<Turn>)> = Vec::new();
    let mut chats = 0;
    let mut left_out = 0;
    for (meta, path) in listed {
        if !super::echo::is_owner_chat(&meta) {
            left_out += 1;
            continue;
        }
        // A file whose time cannot be read is unknown, never outside the
        // window: counted with the unreadable.
        match std::fs::metadata(&path).and_then(|m| m.modified()) {
            Ok(t) if t >= since => {}
            Ok(_) => continue,
            Err(_) => {
                unreadable += 1;
                continue;
            }
        }
        match session_turns(&path) {
            Ok(parts) => {
                for part in parts {
                    all.push((part.at.unwrap_or(meta.created_at), chats, part.turns));
                }
                chats += 1;
            }
            Err(_) => unreadable += 1,
        }
    }
    // Stable, so a chat's own stretches keep their order on a tie.
    all.sort_by_key(|(at, _, _)| *at);
    Ok(Said {
        stretches: all
            .into_iter()
            .map(|(_, chat, turns)| (chat, turns))
            .collect(),
        chats,
        unreadable,
        left_out,
    })
}

/// One persona's reading over a window.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct Initiative {
    /// Chats read.
    pub chats: usize,
    /// People searched for. None means there is nothing to read, not a
    /// reading of zero.
    pub people: usize,
    /// One row per arm with a reply, ordered by days, people and events.
    pub arms: Vec<ArmReading>,
    /// Session files that could not be read: counted, never silently an
    /// empty chat.
    pub unreadable: usize,
    /// Test or withdrawn sessions left out, counted.
    pub left_out: usize,
}

/// One arm's row.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ArmReading {
    #[serde(flatten)]
    pub arm: Arm,
    #[serde(flatten)]
    pub reading: Reading,
    pub initiative: Option<f64>,
    pub uptake: Option<f64>,
}

/// The reading of every chat in `sessions` (a persona's `sessions/`) last
/// written at or after `since`, searching for the people in her network at
/// `pdir`. No folder is no chats, and no people is no reading.
pub fn initiative(
    pdir: &Path,
    sessions: &Path,
    since: std::time::SystemTime,
) -> Result<Initiative> {
    let who = subjects(&Network::read(pdir)?);
    if who.is_empty() {
        let (_, unreadable) = Session::list_counting(sessions)?;
        return Ok(Initiative {
            unreadable,
            ..Initiative::default()
        });
    }
    let said = stretches(sessions, since)?;
    let mut out = Initiative {
        chats: said.chats,
        people: who.len(),
        unreadable: said.unreadable,
        left_out: said.left_out,
        ..Initiative::default()
    };
    out.arms = score(&said.stretches, &who)
        .into_iter()
        .map(|(arm, reading)| ArmReading {
            arm,
            reading,
            initiative: reading.initiative(),
            uptake: reading.uptake(),
        })
        .collect();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pell() -> Vec<Subject> {
        vec![Subject::new(["Pell"]), Subject::new(["Ines", "Nessa"])]
    }

    const OFF: Arm = Arm {
        days: false,
        people: false,
        events: false,
    };
    const ON: Arm = Arm {
        days: true,
        people: true,
        events: true,
    };

    fn turn(owner: &str, reply: &str, arm: Arm) -> Turn {
        Turn {
            owner: owner.into(),
            reply: Some(reply.into()),
            completed: true,
            arm,
        }
    }

    /// Each chat as one stretch, oldest first.
    fn chats(chats: &[Vec<Turn>]) -> Vec<(usize, Vec<Turn>)> {
        chats.iter().cloned().enumerate().collect()
    }

    fn only(list: &[Vec<Turn>]) -> Reading {
        let got = score(&chats(list), &pell());
        assert_eq!(got.len(), 1, "{got:?}");
        got[0].1
    }

    #[test]
    fn a_name_is_whole_words_and_hers_keeps_its_capital() {
        let s = Subject::new(["Will"]);
        assert!(names(&words("Will called."), &s, true));
        assert!(names(&words("Will's coat"), &s, true), "a possessive");
        assert!(
            !names(&words("I will bake rye loaves."), &s, true),
            "will is not Will"
        );
        assert!(
            names(&words("did will call?"), &s, false),
            "uncased: a turn typed with no capital"
        );
        assert!(!names(&words("Willow came"), &s, true));
        let two = Subject::new(["Aunt May"]);
        assert!(names(&words("and Aunt May, too"), &two, true));
        assert!(!names(&words("Aunt Rosa may"), &two, true));
        let alias = Subject::new(["Ines", "Nessa"]);
        assert!(names(&words("Nessa says hi"), &alias, true));
    }

    #[test]
    fn an_introduction_taken_up_asked_about_or_let_go() {
        let r = only(&[vec![
            turn("hi", "Pell kept me up all night.", ON),
            turn("oh no, is pell ok?", "He is, and Nessa came by.", ON),
            turn("that's nice", "Anyway, how was your day?", ON),
        ]]);
        assert_eq!(r.replies, 3);
        assert_eq!((r.introducing, r.introductions), (2, 2));
        assert_eq!((r.taken_up, r.asked), (1, 1));
        assert_eq!(r.not_taken_up, 1, "Ines went unmentioned");
        assert_eq!(r.initiative(), Some(2.0 / 3.0));
        assert_eq!(r.uptake(), Some(0.5));
    }

    #[test]
    fn someone_the_owner_named_first_is_not_hers_to_introduce() {
        let r = only(&[vec![
            turn("how's Pell?", "Pell is fine.", ON),
            turn("good", "Pell says hello.", ON),
        ]]);
        assert_eq!((r.replies, r.introductions), (2, 0));
        assert_eq!(r.initiative(), Some(0.0));
        assert_eq!(r.uptake(), None, "nothing introduced has no rate");
    }

    #[test]
    fn a_return_in_a_later_chat_is_uptake_and_her_repeat_is_not() {
        let r = only(&[
            vec![
                turn("hi", "Pell started school.", ON),
                turn("cool", "Mm.", ON),
            ],
            vec![turn("how did Pell's first week go?", "Well!", ON)],
        ]);
        assert_eq!((r.taken_up, r.came_back, r.not_taken_up), (0, 1, 0));
        assert_eq!(r.uptake(), Some(1.0));
        let r = only(&[
            vec![
                turn("hi", "Pell started school.", ON),
                turn("ok", "Mm.", ON),
            ],
            vec![
                turn("hey", "Pell loves it.", ON),
                turn("great, Pell sounds happy", "He is.", ON),
            ],
        ]);
        // The first went untaken; the second was taken up in its next turn.
        assert_eq!(
            (r.introductions, r.taken_up, r.came_back, r.not_taken_up),
            (2, 1, 0, 1)
        );
    }

    #[test]
    fn a_chats_last_introduction_is_unknown_until_a_return_settles_it() {
        let last = vec![turn("hi", "Pell is home.", ON)];
        let r = only(std::slice::from_ref(&last));
        assert_eq!((r.introductions, r.unanswered), (1, 1));
        assert_eq!(r.uptake(), None, "unknown is never a no");
        let r = only(&[last.clone(), vec![turn("is Pell better?", "Yes.", ON)]]);
        assert_eq!((r.came_back, r.unanswered), (1, 0));
        // The owner spoke in a later chat and never named him: a no, not
        // unknown (review of #808).
        let r = only(&[last, vec![turn("rainy here", "Same.", ON)]]);
        assert_eq!((r.not_taken_up, r.unanswered), (1, 0));
        assert_eq!(r.uptake(), Some(0.0));
        // The owner naming them in the same chat later is not a return.
        let r = only(&[vec![
            turn("hi", "Pell is home.", ON),
            turn("ok", "Mm.", ON),
            turn("wait, Pell?", "Yes.", ON),
        ]]);
        assert_eq!((r.came_back, r.not_taken_up), (0, 1));
    }

    #[test]
    fn a_stopped_reply_names_but_is_not_scored() {
        let mut cut = turn("hi", "Pell", ON);
        cut.completed = false;
        let r = only(&[vec![cut, turn("hm", "Pell again.", ON)]]);
        assert_eq!((r.replies, r.introductions), (1, 0));
    }

    #[test]
    fn arms_are_read_apart() {
        let got = score(
            &chats(&[
                vec![turn("hi", "Pell!", OFF)],
                vec![turn("hi", "Pell!", ON), turn("Pell?", "Yes", ON)],
            ]),
            &pell(),
        );
        assert_eq!(
            got.iter().map(|(a, _)| *a).collect::<Vec<_>>(),
            vec![OFF, ON]
        );
        assert_eq!(got[0].1.introductions, 1);
        // The off arm's Pell was settled by her bringing him up again.
        assert_eq!(got[0].1.not_taken_up, 1);
        assert_eq!(got[1].1.taken_up, 1);
    }

    #[test]
    fn the_owner_is_held_to_capitals_once_they_type_any() {
        // "I will bake rye" has a capital, so its will is not Will; an
        // all-lowercase turn is read case-blind (review of #808).
        let will = vec![Subject::new(["Will"])];
        let r = score(
            &chats(&[vec![
                turn("hi", "Will is visiting.", ON),
                turn("I will bake rye loaves at noon", "Ok.", ON),
            ]]),
            &will,
        );
        assert_eq!((r[0].1.taken_up, r[0].1.not_taken_up), (0, 1));
        let r = score(
            &chats(&[vec![
                turn("hi", "Will is visiting.", ON),
                turn("oh is will staying long", "A week.", ON),
            ]]),
            &will,
        );
        assert_eq!(r[0].1.taken_up, 1);
    }

    #[test]
    fn a_resumed_chat_is_read_where_it_happened() {
        // Chat 0 introduces Pell and stops; chat 1 comes after; then chat 0
        // is resumed and the owner asks about him: that is the owner's next
        // turn in chat 0, so taken up, not a return.
        let got = score(
            &[
                (0, vec![turn("hi", "Pell is home.", ON)]),
                (1, vec![turn("rain again", "Ugh.", ON)]),
                (0, vec![turn("how is Pell?", "Better.", ON)]),
            ],
            &pell(),
        );
        let r = got[0].1;
        assert_eq!((r.came_back, r.taken_up, r.introductions), (0, 1, 1));
        // Asked about in chat 1 instead, while chat 0 waits: a return.
        let got = score(
            &[
                (0, vec![turn("hi", "Pell is home.", ON)]),
                (1, vec![turn("is Pell ok?", "Yes.", ON)]),
            ],
            &pell(),
        );
        assert_eq!((got[0].1.came_back, got[0].1.unanswered), (1, 0));
        // On resuming, the owner's first turn passes him by; her raising
        // him again in the same chat is no introduction, and the owner
        // naming him after that is not a return.
        let got = score(
            &[
                (0, vec![turn("hi", "Pell is home.", ON)]),
                (
                    0,
                    vec![
                        turn("back", "Pell slept well.", ON),
                        turn("good, Pell needed it", "Yes.", ON),
                    ],
                ),
            ],
            &pell(),
        );
        let r = got[0].1;
        assert_eq!((r.introductions, r.not_taken_up, r.came_back), (1, 1, 0));
    }

    #[test]
    fn where_a_chat_splits_changes_no_outcome() {
        // A config recorded mid-chat (a model switch) splits it into
        // stretches; every reading must match the unsplit chat's (review
        // of #808).
        let script = [
            turn("hi", "Pell is home.", ON),
            turn("ok", "Mm.", ON),
            turn("wait, Pell?", "Yes.", ON),
            turn("tell me", "Nessa visited.", ON),
            turn("Nessa! how is she?", "Well.", ON),
            turn("and then", "Pell slept.", ON),
        ];
        let whole = score(&[(0, script.to_vec())], &pell());
        for cut in 1..script.len() {
            let split = score(
                &[(0, script[..cut].to_vec()), (0, script[cut..].to_vec())],
                &pell(),
            );
            assert_eq!(split, whole, "split at {cut}");
        }
        let r = whole[0].1;
        assert_eq!((r.introductions, r.taken_up, r.not_taken_up), (2, 1, 1));
    }

    mod transcripts {
        use super::super::*;
        use crate::agent::{Conversation, StopCause};
        use crate::message::{Block, Message};
        use crate::session::{RunConfig, RunStats, SessionMeta};

        fn scratch(dir: &Path) -> Session {
            scratch_kind(dir, None)
        }

        fn scratch_kind(dir: &Path, kind: Option<crate::session::SessionKind>) -> Session {
            let meta = SessionMeta {
                id: format!("20260101T000000-{}", uuid::Uuid::new_v4().simple()),
                created_at: chrono::Utc::now(),
                provider: "scripted".into(),
                model: "test-model".into(),
                workspace: dir.to_path_buf(),
                title: None,
                kind,
            };
            Session::create(dir, meta).unwrap()
        }

        fn config(s: &Session, prompt: &str, at: &str) {
            s.append(&Record::Config(RunConfig {
                system_prompt: Some(prompt.into()),
                clock: Some(at.parse().unwrap()),
                ..RunConfig::default()
            }))
            .unwrap();
        }

        fn flat(path: &Path) -> Vec<Turn> {
            session_turns(path)
                .unwrap()
                .into_iter()
                .flat_map(|s| s.turns)
                .collect()
        }

        fn run(s: &Session, convo: &mut Conversation, said: &str, answer: &str, notes: &[&str]) {
            let before = convo.messages.clone();
            convo.push(Message::user(said));
            convo.push(Message::assistant(vec![Block::text(answer)]));
            s.record_run(&before, convo).unwrap();
            if !notes.is_empty() {
                s.append(&Record::Notes {
                    notes: notes.iter().map(|n| n.to_string()).collect(),
                })
                .unwrap();
            }
            s.append(&Record::Outcome(RunStats {
                stop_cause: Some(StopCause::Completed),
                ..RunStats::default()
            }))
            .unwrap();
        }

        #[test]
        fn a_transcript_files_each_turn_by_what_she_was_shown() {
            let dir = std::env::temp_dir().join(format!("mecha-init-{}", uuid::Uuid::new_v4()));
            let s = scratch(&dir);
            config(&s, "# Who you are\n\nMara.", "2026-01-01T09:00:00Z");
            let mut convo = Conversation::default();
            run(&s, &mut convo, "hi", "Pell is up early.", &[]);
            let today = format!(
                "{} you are reading. {} today: a small win.)",
                super::super::super::today::TODAY_STEM,
                super::super::super::today::EVENTS_HEADING,
            );
            config(
                &s,
                &format!(
                    "# Who you are\n\nMara.\n\n{}\n\nPell.",
                    super::super::super::agent::PEOPLE_HEADING
                ),
                "2026-01-03T09:00:00Z",
            );
            run(&s, &mut convo, "is Pell ok?", "Yes.", &[&today]);
            // Two stretches, split at the resume, each at its clock.
            let parts = session_turns(&s.path).unwrap();
            assert_eq!(
                parts
                    .iter()
                    .map(|p| (p.at.unwrap().to_rfc3339(), p.turns.len()))
                    .collect::<Vec<_>>(),
                vec![
                    ("2026-01-01T09:00:00+00:00".to_string(), 1),
                    ("2026-01-03T09:00:00+00:00".to_string(), 1)
                ]
            );
            let turns = flat(&s.path);
            assert_eq!(turns.len(), 2);
            assert_eq!(turns[0].arm, Arm::default());
            assert_eq!(
                turns[1].arm,
                Arm {
                    days: true,
                    people: true,
                    events: true,
                }
            );
            assert_eq!(turns[1].owner, "is Pell ok?");
            assert_eq!(turns[0].reply.as_deref(), Some("Pell is up early."));
            // A compaction's rewrite still yields the run's own words.
            let before = convo.messages.clone();
            convo.messages = vec![
                Message::user(format!("{} earlier turns.", crate::compact::SUMMARY_HEADER)),
                Message::user("tell me about Ines"),
                Message::assistant(vec![Block::text("Ines bakes.")]),
            ];
            s.record_run(&before, &convo).unwrap();
            s.append(&Record::Outcome(RunStats {
                stop_cause: Some(StopCause::Completed),
                ..RunStats::default()
            }))
            .unwrap();
            let turns = flat(&s.path);
            assert_eq!(turns[2].owner, "tell me about Ines");
            assert_eq!(turns[2].reply.as_deref(), Some("Ines bakes."));
            // A run that spoke, called a tool and spoke again is read
            // whole, appended or rewritten (review of #808).
            let hop = |convo: &mut Conversation| {
                convo.push(Message::user("and Pell?"));
                convo.push(Message::assistant(vec![
                    Block::text("Pell kept me up."),
                    Block::ToolUse {
                        id: "t1".into(),
                        name: "persona_recall".into(),
                        input: serde_json::json!({}),
                    },
                ]));
                convo.push(Message::tool_results(vec![Block::ToolResult {
                    tool_use_id: "t1".into(),
                    content: "nothing".into(),
                    is_error: false,
                }]));
                convo.push(Message::assistant(vec![Block::text("Anyway.")]));
            };
            let before = convo.messages.clone();
            hop(&mut convo);
            s.record_run(&before, &convo).unwrap();
            s.append(&Record::Outcome(RunStats {
                stop_cause: Some(StopCause::Completed),
                ..RunStats::default()
            }))
            .unwrap();
            let before = convo.messages.clone();
            convo.messages = vec![Message::user(format!(
                "{} earlier turns.",
                crate::compact::SUMMARY_HEADER
            ))];
            hop(&mut convo);
            s.record_run(&before, &convo).unwrap();
            s.append(&Record::Outcome(RunStats {
                stop_cause: Some(StopCause::Completed),
                ..RunStats::default()
            }))
            .unwrap();
            let turns = flat(&s.path);
            for t in &turns[3..5] {
                assert_eq!(t.owner, "and Pell?");
                assert_eq!(t.reply.as_deref(), Some("Pell kept me up.\nAnyway."));
            }
            // A cut past the owner's message: the rewrite opens on the
            // rebuilt head, whose summary names Pell. That is not the
            // owner speaking, and the run's own words are unknown.
            let before = convo.messages.clone();
            let mut head = Message::user("hello");
            head.content.push(Block::text(format!(
                "\n\n{} They talked about Pell.",
                crate::compact::SUMMARY_HEADER
            )));
            convo.messages = vec![head, Message::assistant(vec![Block::text("Mm.")])];
            s.record_run(&before, &convo).unwrap();
            s.append(&Record::Outcome(RunStats {
                stop_cause: Some(StopCause::Completed),
                ..RunStats::default()
            }))
            .unwrap();
            let turns = flat(&s.path);
            assert_eq!(turns[5].owner, "");
            assert_eq!(turns[5].reply.as_deref(), Some("Mm."));
            // A failed turn, recorded as the persona door records one: the
            // owner's message appended, the turn rolled back, no outcome.
            // The rollback re-reads nothing; the owner's words join the turn
            // that answered them.
            let before_fail = convo.messages.clone();
            let asked = Message::user("is Pell ok?");
            s.append(&Record::Message(asked.clone())).unwrap();
            let mut with_ask = before_fail.clone();
            with_ask.push(asked);
            let rolled = Conversation {
                messages: before_fail.clone(),
                ..Conversation::default()
            };
            s.record_run(&with_ask, &rolled).unwrap();
            let mut convo = rolled;
            run(&s, &mut convo, "and you?", "Fine.", &[]);
            let turns = flat(&s.path);
            assert_eq!(turns.len(), 7, "the failed turn is no turn of its own");
            assert_eq!(turns[6].owner, "is Pell ok?\nand you?");
            assert_eq!(turns[6].reply.as_deref(), Some("Fine."));
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn an_old_arm_defaults_shown_events_off() {
            let arm: Arm =
                serde_json::from_str(r#"{"shown_days":true,"shown_people":false}"#).unwrap();
            assert!(arm.days);
            assert!(!arm.people);
            assert!(!arm.events);
        }

        #[test]
        fn stretches_are_read_in_the_order_they_were_said() {
            let dir = std::env::temp_dir().join(format!("mecha-init-{}", uuid::Uuid::new_v4()));
            // Chat a began first and was resumed after chat b.
            let a = scratch(&dir);
            config(&a, "p", "2026-01-01T09:00:00Z");
            let mut ca = Conversation::default();
            run(&a, &mut ca, "hi", "Pell is home.", &[]);
            let b = scratch(&dir);
            config(&b, "p", "2026-01-02T09:00:00Z");
            run(&b, &mut Conversation::default(), "rain", "Ugh.", &[]);
            config(&a, "p", "2026-01-03T09:00:00Z");
            run(&a, &mut ca, "how is Pell?", "Better.", &[]);
            // A test run and a chat the owner withdrew are not her chats:
            // left out and counted (review of #808).
            for kind in [
                crate::session::SessionKind::Test,
                crate::session::SessionKind::Experiment,
                crate::session::SessionKind::Driven,
            ] {
                let t = scratch_kind(&dir, Some(kind));
                config(&t, "p", "2026-01-02T12:00:00Z");
                run(
                    &t,
                    &mut Conversation::default(),
                    "is Pell ok?",
                    "Pell is fine.",
                    &[],
                );
            }
            let read = stretches(&dir, std::time::UNIX_EPOCH).unwrap();
            assert_eq!((read.chats, read.unreadable, read.left_out), (2, 0, 3));
            let said = read.stretches;
            let owners: Vec<&str> = said.iter().map(|(_, t)| t[0].owner.as_str()).collect();
            assert_eq!(owners, vec!["hi", "rain", "how is Pell?"]);
            assert_eq!(said[0].0, said[2].0, "one chat, two stretches");
            assert_ne!(said[0].0, said[1].0);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
