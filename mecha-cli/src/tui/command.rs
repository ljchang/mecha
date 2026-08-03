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
    /// `None` opens the server list; `Some` turns *every* server on or off.
    Mcp(Option<bool>),
    /// One server by name. `None` flips whatever it currently is — the useful
    /// default once there is more than one server and you only care about one.
    McpServer(String, Option<bool>),
    /// A mode was named that does not exist.
    BadMode(String),
    /// An on/off argument that was neither.
    BadToggle(String),
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
        "mcp" => match arg {
            None => Command::Mcp(None),
            Some(a) => {
                let mut words = a.split_whitespace();
                let first = words.next().unwrap_or("");
                let second = words.next();
                match (parse_toggle(first), second) {
                    // `/mcp off` — everything.
                    (Some(v), None) => Command::Mcp(Some(v)),
                    // `/mcp off pkg` reads naturally but is the wrong way
                    // round; say so rather than guessing which was meant.
                    (Some(_), Some(_)) => Command::BadToggle(a.to_string()),
                    // `/mcp pkg` — flip that one.
                    (None, None) => Command::McpServer(first.to_string(), None),
                    // `/mcp pkg off`.
                    (None, Some(word)) => match parse_toggle(word) {
                        Some(v) => Command::McpServer(first.to_string(), Some(v)),
                        None => Command::BadToggle(word.to_string()),
                    },
                }
            }
        },
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

fn parse_toggle(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "on" | "yes" | "true" | "enable" => Some(true),
        "off" | "no" | "false" | "disable" => Some(false),
        _ => None,
    }
}

/// Every command name, in the order they are offered for completion.
///
/// One list, so completion and `HELP` cannot drift apart — there is a test that
/// every name here parses, and another that everything `HELP` advertises is
/// here.
pub const NAMES: [&str; 9] =
    ["help", "tools", "model", "provider", "mode", "mcp", "usage", "clear", "session"];

/// Command names that could still be meant by what has been typed.
///
/// Empty for anything that is not a command being typed: once there is
/// whitespace the name is settled and the user is onto arguments, and once
/// there is an exact match there is nothing left to suggest.
pub fn completions(input: &str) -> Vec<&'static str> {
    let Some(rest) = input.strip_prefix('/') else { return Vec::new() };
    if rest.contains(char::is_whitespace) {
        return Vec::new();
    }
    let rest = rest.to_ascii_lowercase();
    NAMES.iter().copied().filter(|n| n.starts_with(&rest) && *n != rest).collect()
}

/// The longest prefix every candidate shares — what Tab should fill in.
///
/// Completing to the *common* prefix rather than the first match is what makes
/// repeated Tab presses converge instead of cycling through guesses.
pub fn common_prefix(candidates: &[&str]) -> String {
    let Some(first) = candidates.first() else { return String::new() };
    let mut len = first.len();
    for c in &candidates[1..] {
        len = len.min(
            first
                .chars()
                .zip(c.chars())
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| a.len_utf8())
                .sum(),
        );
    }
    first[..len].to_string()
}

pub fn mode_name(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Ask => "ask",
        PermissionMode::Allow => "allow",
        PermissionMode::ReadOnly => "read-only",
    }
}

pub const HELP: &str = "\
  /help                  this list
  /tools                 tools this agent can call
  /model [id]            show or switch the model
  /provider [name]       show or switch the provider
  /mode [ask|allow|read-only]   show or switch the permission mode
  /mcp [on|off]          list MCP servers, or turn them all off and on
  /mcp <server> [on|off] turn one server off and on
  /usage                 tokens used this session
  /clear                 start a new conversation, dropping its taint
  /session               where the transcript is being written
  /exit                  quit";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_only_fires_while_the_name_is_still_being_typed() {
        assert_eq!(completions("/mo"), vec!["model", "mode"]);
        assert_eq!(completions("/mod"), vec!["model", "mode"]);
        assert_eq!(completions("/mode"), vec!["model"], "an exact match still offers longer names");
        assert_eq!(completions("/c"), vec!["clear"]);

        // Not a command, or past the name: nothing to suggest.
        assert!(completions("summarise this").is_empty());
        assert!(completions("/model claude").is_empty(), "arguments are not command names");
        assert!(completions("/zzz").is_empty());
    }

    #[test]
    fn tab_fills_in_what_every_candidate_agrees_on() {
        // `/mo` -> `mode`, the longest prefix "model" and "mode" agree on.
        // Completing to the first match instead would make a second Tab undo
        // the first.
        assert_eq!(common_prefix(&completions("/mo")), "mode");
        assert_eq!(common_prefix(&["session", "settings"]), "se");
        assert_eq!(common_prefix(&completions("/u")), "usage");
        assert_eq!(common_prefix(&[]), "");
    }

    #[test]
    fn the_name_list_and_the_help_text_cannot_drift_apart() {
        for name in NAMES {
            assert!(
                !matches!(parse(&format!("/{name}")), None | Some(Command::Unknown(_))),
                "{name} is offered for completion but does not parse"
            );
        }
        for line in HELP.lines() {
            let Some(advertised) = line.split_whitespace().next() else { continue };
            let advertised = advertised.trim_start_matches('/');
            if advertised == "exit" {
                continue; // an alias, deliberately not offered first
            }
            assert!(NAMES.contains(&advertised), "{advertised} is documented but not completable");
        }
    }

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
    fn mcp_toggles_on_the_words_people_actually_type() {
        for word in ["on", "yes", "true", "enable", "ON"] {
            assert_eq!(parse(&format!("/mcp {word}")), Some(Command::Mcp(Some(true))), "{word}");
        }
        for word in ["off", "no", "false", "disable"] {
            assert_eq!(parse(&format!("/mcp {word}")), Some(Command::Mcp(Some(false))), "{word}");
        }
        assert_eq!(parse("/mcp"), Some(Command::Mcp(None)));
        // A single word that is not on/off is a server name — the parser
        // cannot know which servers exist, so an unknown one is caught at
        // dispatch, where the configured list can be shown.
        assert_eq!(parse("/mcp maybe"), Some(Command::McpServer("maybe".into(), None)));
    }

    #[test]
    fn mcp_addresses_all_the_servers_or_one_of_them() {
        assert_eq!(parse("/mcp off"), Some(Command::Mcp(Some(false))));
        assert_eq!(parse("/mcp pkg off"), Some(Command::McpServer("pkg".into(), Some(false))));
        assert_eq!(parse("/mcp pkg on"), Some(Command::McpServer("pkg".into(), Some(true))));
        // A bare name flips it, which is what you want when there is one
        // server you keep reaching for.
        assert_eq!(parse("/mcp pkg"), Some(Command::McpServer("pkg".into(), None)));

        // Reads naturally, means the opposite of what it looks like. Refused
        // rather than guessed at.
        assert_eq!(parse("/mcp off pkg"), Some(Command::BadToggle("off pkg".into())));
        assert_eq!(parse("/mcp pkg maybe"), Some(Command::BadToggle("maybe".into())));
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
