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
//! at all, as a phone often is ("is milo ok"), where case is ignored
//! (review of #808). A name that is also a common word still over-counts
//! where a sentence starts with it.
//!
//! - **An introduction** is a completed reply naming someone neither side
//!   has named earlier in that chat. Harness notes are not read, only what
//!   the owner typed and what she said, so her people riding in her prompt
//!   do not count as either side naming them.
//! - **Taken up** is the owner naming them in the next turn they speak
//!   in, and **asked** the subset where that turn holds a question mark.
//! - **Came back** is the owner naming them in a later stretch of talk
//!   before she does: another chat, or the same one resumed.
//! - **Not taken up** is her bringing them up again first, or the owner
//!   speaking after the introduction without ever naming them: in the
//!   next turn, or, for a chat's last introduction, in any later chat.
//!   A chat usually ends on her turn, so that last case is the common one,
//!   and leaving it out would flatter the rate (review of #808).
//! - **Unanswered** is an introduction the owner has not spoken after at
//!   all: the newest chat's last turn. Its outcome is unknown, so it is
//!   kept out of the rate, never counted as a no.
//!
//! **Order is when it was said.** "Later" is read in time, not by chat. A
//! chat resumed days after it began is read as stretches, each from its
//! start or a resume (where the transcript records a config, with its
//! clock) to the next, and every chat's stretches are read in one order by
//! that clock. Ordering whole chats by when they began would put a
//! long-lived chat's newest turns before chats that came after them, and
//! each misplaced turn errs toward uptake (review of #808).
//!
//! **Arms, from what she was shown, not from her settings.** Each turn is
//! filed by whether the transcript shows her days (a today-block among the
//! run's notes) and her people (their heading in the recorded system
//! prompt). Settings change, and a chat keeps the version it began with,
//! so the record is the only honest source.
//!
//! **Counts, never words.** A reading holds numbers only. A name is
//! looked for, never kept, so a reading can be pasted anywhere.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize)]
pub struct Arm {
    /// A today-block among the run's notes.
    pub days: bool,
    /// Her people in the recorded system prompt.
    pub people: bool,
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
/// messages (appended, or a `Rewrite` holding the whole history when it
/// compacted), its notes, then its `Outcome`. The owner's text is every
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
                if m.role == Role::User {
                    let said = crate::agent::owner_text(&m);
                    if !said.trim().is_empty() {
                        if !run.owner.is_empty() {
                            run.owner.push('\n');
                        }
                        run.owner.push_str(&said);
                    }
                } else if let Some(reply) = reply_text(&m) {
                    run.reply = Some(reply);
                }
            }
            Ok(Record::Rewrite { messages }) => {
                if let Some(reply) = messages.last().and_then(reply_text) {
                    run.reply = Some(reply);
                }
                if let Some(said) = messages
                    .iter()
                    .rev()
                    .filter(|m| m.role == Role::User)
                    .map(crate::agent::owner_text)
                    .find(|t| !t.trim().is_empty())
                {
                    run.owner = said;
                }
            }
            Ok(Record::Notes { notes }) => {
                if notes.iter().any(|n| super::today::stem_of(n).is_some()) {
                    run.arm.days = true;
                }
            }
            Ok(Record::Outcome(stats)) => {
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

    /// Of the introductions with an outcome, the share the owner took up,
    /// in the next turn or a later chat; `None` when none has an outcome.
    pub fn uptake(&self) -> Option<f64> {
        let decided = self.taken_up + self.came_back + self.not_taken_up;
        (decided > 0).then(|| (self.taken_up + self.came_back) as f64 / decided as f64)
    }
}

/// An introduction waiting on its outcome.
struct Waiting {
    arm: Arm,
    /// The stretch it was made in, which a return must come after.
    stretch: usize,
    /// The owner had a next turn in its stretch and did not name them.
    passed: bool,
}

/// How a waiting introduction ended.
enum Settled {
    /// The owner brought them up first in a later stretch.
    Back,
    /// She brought them up again first.
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

/// Every arm's reading over `stretches`, each `(chat, turns)`, in the
/// order they were said, searching for `who`. Arms with no reply are left
/// out.
pub fn score(stretches: &[(usize, Vec<Turn>)], who: &[Subject]) -> Vec<(Arm, Reading)> {
    let mut by: HashMap<Arm, Reading> = HashMap::new();
    // Per subject, introductions not taken up in the next turn, waiting
    // for a return in a later stretch.
    let mut waiting: HashMap<usize, Vec<Waiting>> = HashMap::new();
    // Named by either side earlier in each chat, across its stretches.
    let mut named: HashMap<usize, HashSet<usize>> = HashMap::new();
    // The last stretch the owner spoke in.
    let mut spoke: Option<usize> = None;
    // A return or a repeat settles what an earlier stretch left waiting.
    let settle_earlier = |by: &mut HashMap<Arm, Reading>,
                          ws: &mut Vec<Waiting>,
                          now: usize,
                          how: fn() -> Settled| {
        let (done, keep): (Vec<_>, Vec<_>) = ws.drain(..).partition(|w| w.stretch < now);
        *ws = keep;
        for w in done {
            settle(by, w, how());
        }
    };
    for (stretch, (chat, turns)) in stretches.iter().enumerate() {
        let named = named.entry(*chat).or_default();
        // Introduced in the turn before, waiting on the owner's next turn.
        let mut last: Vec<(usize, Arm)> = Vec::new();
        for turn in turns {
            let owner = words(&turn.owner);
            if !owner.is_empty() {
                spoke = Some(stretch);
                let asked = turn.owner.contains('?');
                let cased = turn.owner.chars().any(char::is_uppercase);
                for (s, arm) in last.drain(..) {
                    if names(&owner, &who[s], cased) {
                        let r = by.entry(arm).or_default();
                        r.taken_up += 1;
                        if asked {
                            r.asked += 1;
                        }
                    } else {
                        waiting.entry(s).or_default().push(Waiting {
                            arm,
                            stretch,
                            passed: true,
                        });
                    }
                }
                for (s, subject) in who.iter().enumerate() {
                    if !names(&owner, subject, cased) {
                        continue;
                    }
                    named.insert(s);
                    if let Some(ws) = waiting.get_mut(&s) {
                        settle_earlier(&mut by, ws, stretch, || Settled::Back);
                    }
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
                // Her bringing them up again first settles what an
                // earlier stretch left waiting, a resumed chat's included.
                if let Some(ws) = waiting.get_mut(&s) {
                    settle_earlier(&mut by, ws, stretch, || Settled::Repeated);
                }
                if !named.insert(s) {
                    continue;
                }
                if turn.completed {
                    introduced = true;
                    by.entry(turn.arm).or_default().introductions += 1;
                    last.push((s, turn.arm));
                }
            }
            if introduced {
                by.entry(turn.arm).or_default().introducing += 1;
            }
        }
        // The stretch's last introductions had no next turn: they wait for
        // a later stretch, and are unanswered only if the owner never
        // speaks again.
        for (s, arm) in last {
            waiting.entry(s).or_default().push(Waiting {
                arm,
                stretch,
                passed: false,
            });
        }
    }
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
}

/// Every stretch of every chat in `sessions` last written at or after
/// `since`, in the order they were said: by the stretch's clock, or its
/// chat's start where none was recorded.
pub fn stretches(sessions: &Path, since: std::time::SystemTime) -> Result<Said> {
    let (listed, mut unreadable) = Session::list_counting(sessions)?;
    let mut all: Vec<(DateTime<Utc>, usize, Vec<Turn>)> = Vec::new();
    let mut chats = 0;
    for (meta, path) in listed {
        let active = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t >= since);
        if !active {
            continue;
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
    /// One row per arm with a reply, days off and people off first.
    pub arms: Vec<ArmReading>,
    /// Session files that could not be read: counted, never silently an
    /// empty chat.
    pub unreadable: usize,
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

    fn milo() -> Vec<Subject> {
        vec![Subject::new(["Milo"]), Subject::new(["Ines", "Nessa"])]
    }

    const OFF: Arm = Arm {
        days: false,
        people: false,
    };
    const ON: Arm = Arm {
        days: true,
        people: true,
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
        let got = score(&chats(list), &milo());
        assert_eq!(got.len(), 1, "{got:?}");
        got[0].1
    }

    #[test]
    fn a_name_is_whole_words_and_hers_keeps_its_capital() {
        let s = Subject::new(["Will"]);
        assert!(names(&words("Will called."), &s, true));
        assert!(names(&words("Will's coat"), &s, true), "a possessive");
        assert!(!names(&words("I will call."), &s, true), "will is not Will");
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
            turn("hi", "Milo kept me up all night.", ON),
            turn("oh no, is milo ok?", "He is, and Nessa came by.", ON),
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
            turn("how's Milo?", "Milo is fine.", ON),
            turn("good", "Milo says hello.", ON),
        ]]);
        assert_eq!((r.replies, r.introductions), (2, 0));
        assert_eq!(r.initiative(), Some(0.0));
        assert_eq!(r.uptake(), None, "nothing introduced has no rate");
    }

    #[test]
    fn a_return_in_a_later_chat_is_uptake_and_her_repeat_is_not() {
        let r = only(&[
            vec![
                turn("hi", "Milo started school.", ON),
                turn("cool", "Mm.", ON),
            ],
            vec![turn("how did Milo's first week go?", "Well!", ON)],
        ]);
        assert_eq!((r.taken_up, r.came_back, r.not_taken_up), (0, 1, 0));
        assert_eq!(r.uptake(), Some(1.0));
        let r = only(&[
            vec![
                turn("hi", "Milo started school.", ON),
                turn("ok", "Mm.", ON),
            ],
            vec![
                turn("hey", "Milo loves it.", ON),
                turn("great, Milo sounds happy", "He is.", ON),
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
        let last = vec![turn("hi", "Milo is home.", ON)];
        let r = only(std::slice::from_ref(&last));
        assert_eq!((r.introductions, r.unanswered), (1, 1));
        assert_eq!(r.uptake(), None, "unknown is never a no");
        let r = only(&[last.clone(), vec![turn("is Milo better?", "Yes.", ON)]]);
        assert_eq!((r.came_back, r.unanswered), (1, 0));
        // The owner spoke in a later chat and never named him: a no, not
        // unknown (review of #808).
        let r = only(&[last, vec![turn("rainy here", "Same.", ON)]]);
        assert_eq!((r.not_taken_up, r.unanswered), (1, 0));
        assert_eq!(r.uptake(), Some(0.0));
        // The owner naming them in the same chat later is not a return.
        let r = only(&[vec![
            turn("hi", "Milo is home.", ON),
            turn("ok", "Mm.", ON),
            turn("wait, Milo?", "Yes.", ON),
        ]]);
        assert_eq!((r.came_back, r.not_taken_up), (0, 1));
    }

    #[test]
    fn a_stopped_reply_names_but_is_not_scored() {
        let mut cut = turn("hi", "Milo", ON);
        cut.completed = false;
        let r = only(&[vec![cut, turn("hm", "Milo again.", ON)]]);
        assert_eq!((r.replies, r.introductions), (1, 0));
    }

    #[test]
    fn arms_are_read_apart() {
        let got = score(
            &chats(&[
                vec![turn("hi", "Milo!", OFF)],
                vec![turn("hi", "Milo!", ON), turn("Milo?", "Yes", ON)],
            ]),
            &milo(),
        );
        assert_eq!(
            got.iter().map(|(a, _)| *a).collect::<Vec<_>>(),
            vec![OFF, ON]
        );
        assert_eq!(got[0].1.introductions, 1);
        // The off arm's Milo was settled by her bringing him up again.
        assert_eq!(got[0].1.not_taken_up, 1);
        assert_eq!(got[1].1.taken_up, 1);
    }

    #[test]
    fn the_owner_is_held_to_capitals_once_they_type_any() {
        // "I will call" has a capital, so its will is not Will; an
        // all-lowercase turn is read case-blind (review of #808).
        let will = vec![Subject::new(["Will"])];
        let r = score(
            &chats(&[vec![
                turn("hi", "Will is visiting.", ON),
                turn("I will call you later", "Ok.", ON),
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
        // Chat 0 introduces Milo and stops; chat 1 comes after; then chat 0
        // is resumed and the owner asks about him. Read in time, the
        // owner's question is a return in a later stretch.
        let got = score(
            &[
                (0, vec![turn("hi", "Milo is home.", ON)]),
                (1, vec![turn("rain again", "Ugh.", ON)]),
                (0, vec![turn("how is Milo?", "Better.", ON)]),
            ],
            &milo(),
        );
        assert_eq!(
            (
                got[0].1.came_back,
                got[0].1.taken_up,
                got[0].1.introductions
            ),
            (1, 0, 1)
        );
        // Her raising him first on resuming is a repeat, not an
        // introduction, and settles the wait as a no.
        let got = score(
            &[
                (0, vec![turn("hi", "Milo is home.", ON)]),
                (
                    0,
                    vec![
                        turn("back", "Milo slept well.", ON),
                        turn("good, Milo needed it", "Yes.", ON),
                    ],
                ),
            ],
            &milo(),
        );
        assert_eq!(
            (
                got[0].1.introductions,
                got[0].1.not_taken_up,
                got[0].1.came_back
            ),
            (1, 1, 0)
        );
    }

    mod transcripts {
        use super::super::*;
        use crate::agent::{Conversation, StopCause};
        use crate::message::{Block, Message};
        use crate::session::{RunConfig, RunStats, SessionMeta};

        fn scratch(dir: &Path) -> Session {
            let meta = SessionMeta {
                id: format!("20260101T000000-{}", uuid::Uuid::new_v4().simple()),
                created_at: chrono::Utc::now(),
                provider: "scripted".into(),
                model: "test-model".into(),
                workspace: dir.to_path_buf(),
                title: None,
                kind: None,
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
            run(&s, &mut convo, "hi", "Milo is up early.", &[]);
            let today = format!(
                "{} you are reading.)",
                super::super::super::today::TODAY_STEM
            );
            config(
                &s,
                &format!(
                    "# Who you are\n\nMara.\n\n{}\n\nMilo.",
                    super::super::super::agent::PEOPLE_HEADING
                ),
                "2026-01-03T09:00:00Z",
            );
            run(&s, &mut convo, "is Milo ok?", "Yes.", &[&today]);
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
                    people: true
                }
            );
            assert_eq!(turns[1].owner, "is Milo ok?");
            assert_eq!(turns[0].reply.as_deref(), Some("Milo is up early."));
            // A compaction's rewrite still yields the run's own words.
            let before = convo.messages.clone();
            convo.messages = vec![
                Message::user("[summary]"),
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
            let _ = std::fs::remove_dir_all(&dir);
        }

        #[test]
        fn stretches_are_read_in_the_order_they_were_said() {
            let dir = std::env::temp_dir().join(format!("mecha-init-{}", uuid::Uuid::new_v4()));
            // Chat a began first and was resumed after chat b.
            let a = scratch(&dir);
            config(&a, "p", "2026-01-01T09:00:00Z");
            let mut ca = Conversation::default();
            run(&a, &mut ca, "hi", "Milo is home.", &[]);
            let b = scratch(&dir);
            config(&b, "p", "2026-01-02T09:00:00Z");
            run(&b, &mut Conversation::default(), "rain", "Ugh.", &[]);
            config(&a, "p", "2026-01-03T09:00:00Z");
            run(&a, &mut ca, "how is Milo?", "Better.", &[]);
            let read = stretches(&dir, std::time::UNIX_EPOCH).unwrap();
            assert_eq!((read.chats, read.unreadable), (2, 0));
            let said = read.stretches;
            let owners: Vec<&str> = said.iter().map(|(_, t)| t[0].owner.as_str()).collect();
            assert_eq!(owners, vec!["hi", "rain", "how is Milo?"]);
            assert_eq!(said[0].0, said[2].0, "one chat, two stretches");
            assert_ne!(said[0].0, said[1].0);
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
