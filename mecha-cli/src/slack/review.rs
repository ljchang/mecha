//! Per-thread (and per-channel) release policy: what happens when a run this
//! surface started stages drafts. The Slack counterpart of the TUI's
//! `/review now|later|auto` — and deliberately the same policy, not a copy:
//! the mode enum and the release rule live in [`crate::review_policy`], where
//! both front-ends consume them, so the tainted exclusion and the
//! nothing-releases-after-an-early-stop rule cannot be forgotten on one
//! surface and kept on the other.
//!
//! **Set by an explicit owner gesture only — the `review` command word — and
//! never inferred from prompt or message text.** The rule is `/review`'s,
//! quoted because it is load-bearing: *release policy must not be decidable
//! by anything sharing a context window with third-party text.* A command
//! word is an owner's keystroke that short-circuits before the text can
//! become a prompt (the same precedence `doctor` gets), so no fetched page,
//! no mail body and no model output can utter it into effect; anything that
//! is not exactly the command word falls through and is just a message.
//!
//! **Scope follows where the word was spoken.** Inside a thread, the setting
//! governs that thread. As a *top-level* message it governs the channel's
//! subsequent top-level prompts — a top-level `review auto` used to key
//! itself to its own message's ts, a thread no later message ever joins, so
//! it confirmed a policy that governed nothing. A thread's own setting still
//! wins over its channel's.
//!
//! **Session-scoped, expiring with the connector's in-memory state.** The
//! settings live in the connector's process and are deliberately never
//! written to the thread record — the same eviction that orphans a mid-flight
//! run on restart clears every review mode with it. That is the owner
//! decision of 2026-08-14 (SLACK-ACTIONS-DESIGN §4): not an *unbounded*
//! Always, a mode that dies with the state that watched it get set. A
//! connector restart resets everything to `now`, which is the safe
//! direction: cards for everything.

use std::collections::HashMap;

pub use crate::review_policy::ReviewMode;

/// Who set a mode — the attribution every auto-released item's ledger row
/// carries as its `user_id`; the *when* of each release is the ledger row's
/// own stamp.
#[derive(Debug, Clone)]
pub struct Setting {
    pub mode: ReviewMode,
    /// The Slack user id from the signed payload of the message that set it.
    pub set_by: String,
}

/// The `review` command word: `review` alone asks, `review now|later|auto`
/// sets. Matched like `doctor` — trimmed, case-insensitive, and **nothing
/// longer**: "review the design doc" must reach the model, not the policy.
///
/// Returns `None` when the text is not the command word at all,
/// `Some(None)` for the bare question, `Some(Some(mode))` for a setting.
pub fn command(text: &str) -> Option<Option<ReviewMode>> {
    let mut words = text.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("review") {
        return None;
    }
    match (words.next(), words.next()) {
        (None, _) => Some(None),
        (Some(mode), None) => ReviewMode::parse(mode).map(Some),
        // A third word means prose, and prose is a prompt.
        _ => None,
    }
}

/// The key a channel-scoped setting lives under. Its `:` cannot appear in a
/// thread key (`threads::key_for` maps every non-alphanumeric byte to `-`),
/// so the two scopes cannot collide.
pub fn channel_scope_key(channel: &str) -> String {
    format!("channel:{channel}")
}

/// Where a `review` command's setting lands, and the words the confirmation
/// uses to say so. `in_thread` is the event's **raw** `thread_ts` — present
/// only when the message actually sits inside a thread. A top-level command
/// scopes to the channel: keyed to its own message's ts it would govern a
/// thread no later message ever joins.
pub fn scope_for(channel: &str, in_thread: Option<&str>) -> (String, &'static str) {
    match in_thread {
        Some(ts) => (super::threads::key_for(channel, ts), "this thread"),
        None => (
            channel_scope_key(channel),
            "top-level prompts in this channel",
        ),
    }
}

/// The setting that governs one run's drafts: the thread's own if it has one,
/// else the channel's. Thread beats channel, because the narrower gesture is
/// the later-considered one.
pub fn effective<'a>(
    settings: &'a HashMap<String, Setting>,
    thread_key: &str,
    channel: &str,
) -> Option<&'a Setting> {
    settings
        .get(thread_key)
        .or_else(|| settings.get(&channel_scope_key(channel)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_is_set_by_the_exact_command_word_and_nothing_longer() {
        assert_eq!(command("review auto"), Some(Some(ReviewMode::Auto)));
        assert_eq!(command("  REVIEW Now  "), Some(Some(ReviewMode::Now)));
        assert_eq!(command("review later"), Some(Some(ReviewMode::Later)));
        assert_eq!(command("review"), Some(None), "the bare word asks");

        // Release policy must not be decidable by anything sharing a context
        // window with third-party text: prose that merely contains the words
        // is a prompt, never a gesture — including anything a model or a
        // fetched page could get echoed into the thread.
        for prose in [
            "please review auto tomorrow",
            "review auto now",
            "set review to auto",
            "review the design doc",
            "auto",
            "reviewauto",
            "",
        ] {
            assert_eq!(command(prose), None, "{prose:?} must not set a mode");
        }
        // An unknown mode word is not guessed at.
        assert_eq!(command("review always"), None);
    }

    #[test]
    fn every_mode_names_itself_and_round_trips_through_the_command_word() {
        for mode in [ReviewMode::Now, ReviewMode::Later, ReviewMode::Auto] {
            assert_eq!(command(&format!("review {}", mode.name())), Some(Some(mode)));
            assert!(!mode.describe().is_empty());
        }
    }

    /// F8, failing on the old keying: a top-level `review auto` used to key
    /// itself to its own message's ts — a thread no later message ever joins —
    /// so a later top-level run in the same channel found nothing. Channel
    /// scope is what makes the confirmed policy govern something.
    #[test]
    fn a_top_level_setting_scopes_to_the_channel_and_later_top_level_runs_see_it() {
        let channel = "D1";
        let mut settings: HashMap<String, Setting> = HashMap::new();

        // The owner sends a top-level `review auto` (raw thread_ts: None).
        let (key, scope) = scope_for(channel, None);
        assert_eq!(key, channel_scope_key(channel));
        assert!(scope.contains("channel"), "the confirmation names the real scope: {scope}");
        settings.insert(
            key,
            Setting {
                mode: ReviewMode::Auto,
                set_by: "U_OWNER".into(),
            },
        );

        // A later top-level prompt starts a run whose thread key is its own
        // ts — a key nobody ever set anything under. The channel setting
        // governs it.
        let later_run_key = super::super::threads::key_for(channel, "1755200000.000100");
        let seen = effective(&settings, &later_run_key, channel).expect("the mode governs");
        assert_eq!(seen.mode, ReviewMode::Auto);
    }

    #[test]
    fn a_threads_own_setting_wins_over_its_channels() {
        let channel = "D1";
        let mut settings: HashMap<String, Setting> = HashMap::new();
        settings.insert(
            channel_scope_key(channel),
            Setting {
                mode: ReviewMode::Auto,
                set_by: "U_OWNER".into(),
            },
        );

        // Inside a thread, the word scopes to that thread and overrides.
        let (thread_key, scope) = scope_for(channel, Some("1755100000.000200"));
        assert_eq!(scope, "this thread");
        settings.insert(
            thread_key.clone(),
            Setting {
                mode: ReviewMode::Now,
                set_by: "U_OWNER".into(),
            },
        );
        assert_eq!(
            effective(&settings, &thread_key, channel).unwrap().mode,
            ReviewMode::Now,
            "the narrower gesture wins"
        );

        // A different thread in the channel still sees the channel mode.
        let other = super::super::threads::key_for(channel, "1755100001.000300");
        assert_eq!(
            effective(&settings, &other, channel).unwrap().mode,
            ReviewMode::Auto
        );

        // And the two scopes cannot collide by construction.
        assert!(!thread_key.contains(':'));
        assert!(channel_scope_key(channel).contains(':'));
    }
}
