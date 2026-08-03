//! Slash commands for the TUI.
//!
//! Parsing is separated from doing so it can be tested without a terminal, an
//! agent, or a model. The whole `mecha-cli` crate had no tests when this was
//! written; a pure parser is the cheapest place to start having some.
//!
//! `mecha chat` has had these for a while. The TUI is where they matter more,
//! because it is the interface people actually sit in — and it is the only one
//! that can change anything mid-session, since it owns the event loop.

use mecha_core::config::PermissionMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    Tools,
    /// `None` shows the current model; `Some` switches to it.
    Model(Option<String>),
    Provider(Option<String>),
    /// `None` shows the current permission mode; `Some` switches to it.
    Mode(Option<PermissionMode>),
    Usage,
    Clear,
    Session,
    Quit,
    /// Recognised as a command, but not one we have. Kept as its own variant so
    /// a typo says so instead of being sent to the model as a prompt.
    Unknown(String),
    /// A mode was named that does not exist.
    BadMode(String),
}

/// Parse a line of input as a command, or `None` if it is an ordinary message.
///
/// A bare `/` is not a command — someone typing a path or a regex should not
/// have it swallowed. Nor is `/ foo`, for the same reason.
pub fn parse(line: &str) -> Option<Command> {
    let line = line.trim();
    let rest = line.strip_prefix('/')?;
    let mut parts = rest.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or("").trim();
    let arg = parts.next().map(str::trim).filter(|a| !a.is_empty());

    if name.is_empty() {
        return None;
    }

    Some(match name {
        "help" | "h" | "?" => Command::Help,
        "tools" => Command::Tools,
        "model" | "m" => Command::Model(arg.map(str::to_string)),
        "provider" | "p" => Command::Provider(arg.map(str::to_string)),
        "usage" => Command::Usage,
        "clear" | "new" => Command::Clear,
        "session" => Command::Session,
        "exit" | "quit" | "q" => Command::Quit,
        "mode" => match arg {
            None => Command::Mode(None),
            Some(a) => match parse_mode(a) {
                Some(m) => Command::Mode(Some(m)),
                None => Command::BadMode(a.to_string()),
            },
        },
        other => Command::Unknown(other.to_string()),
    })
}

fn parse_mode(s: &str) -> Option<PermissionMode> {
    match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "ask" => Some(PermissionMode::Ask),
        // `auto` and `yes` because that is what the flag is called, and nobody
        // should have to remember which vocabulary this particular surface uses.
        "allow" | "auto" | "yes" => Some(PermissionMode::Allow),
        "read-only" | "readonly" | "ro" | "plan" => Some(PermissionMode::ReadOnly),
        _ => None,
    }
}

pub const HELP: &str = "\
  /help                  this list
  /tools                 tools this agent can call
  /model [id]            show or switch the model
  /provider [name]       show or switch the provider
  /mode [ask|allow|read-only]   show or switch the permission mode
  /usage                 tokens used this session
  /clear                 start a new conversation, dropping its taint
  /session               where the transcript is being written
  /exit                  quit";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ordinary_message_is_not_a_command() {
        assert_eq!(parse("summarise the README"), None);
        assert_eq!(parse("what does a/b mean?"), None);
    }

    #[test]
    fn a_bare_slash_is_not_a_command() {
        // Someone typing a path or a regex should keep it. Swallowing `/` would
        // make the input line quietly lossy.
        assert_eq!(parse("/"), None);
        assert_eq!(parse("/ "), None);
        assert_eq!(parse("/ read this"), None);
    }

    #[test]
    fn commands_parse_with_and_without_arguments() {
        assert_eq!(parse("/model"), Some(Command::Model(None)));
        assert_eq!(parse("/model claude-opus-5"), Some(Command::Model(Some("claude-opus-5".into()))));
        assert_eq!(parse("/provider local"), Some(Command::Provider(Some("local".into()))));
        assert_eq!(parse("/clear"), Some(Command::Clear));
    }

    #[test]
    fn surrounding_whitespace_never_decides_anything() {
        assert_eq!(parse("  /usage  "), Some(Command::Usage));
        assert_eq!(parse("/model    gpt-4o   "), Some(Command::Model(Some("gpt-4o".into()))));
    }

    #[test]
    fn aliases_exist_because_nobody_remembers_which_word_this_surface_uses() {
        assert_eq!(parse("/q"), Some(Command::Quit));
        assert_eq!(parse("/quit"), Some(Command::Quit));
        assert_eq!(parse("/exit"), Some(Command::Quit));
        assert_eq!(parse("/new"), Some(Command::Clear));
        assert_eq!(parse("/?"), Some(Command::Help));
    }

    #[test]
    fn modes_accept_the_names_the_flags_use() {
        // `--yes` on the command line, "auto" in every other agent UI, `allow`
        // in the config file. All three mean the same thing.
        for word in ["allow", "auto", "yes", "ALLOW"] {
            assert_eq!(parse(&format!("/mode {word}")), Some(Command::Mode(Some(PermissionMode::Allow))));
        }
        for word in ["read-only", "readonly", "ro", "plan"] {
            assert_eq!(
                parse(&format!("/mode {word}")),
                Some(Command::Mode(Some(PermissionMode::ReadOnly))),
                "{word}"
            );
        }
        assert_eq!(parse("/mode ask"), Some(Command::Mode(Some(PermissionMode::Ask))));
        assert_eq!(parse("/mode"), Some(Command::Mode(None)));
    }

    #[test]
    fn an_unknown_mode_is_reported_rather_than_silently_ignored() {
        // Silently keeping the old mode would leave someone believing they had
        // switched to read-only when they had not.
        assert_eq!(parse("/mode turbo"), Some(Command::BadMode("turbo".into())));
    }

    #[test]
    fn a_mistyped_command_does_not_become_a_prompt() {
        // The failure this prevents: `/moel gpt-4o` sailing past as a message
        // and the model gamely trying to answer it.
        assert_eq!(parse("/moel"), Some(Command::Unknown("moel".into())));
        assert_eq!(parse("/clera"), Some(Command::Unknown("clera".into())));
    }

    #[test]
    fn every_command_in_the_help_text_actually_parses() {
        // Help that lists a command nobody implemented is worse than no help.
        for line in HELP.lines() {
            let Some(name) = line.split_whitespace().next() else { continue };
            let parsed = parse(name);
            assert!(
                !matches!(parsed, None | Some(Command::Unknown(_))),
                "{name} is advertised but does not parse"
            );
        }
    }
}
