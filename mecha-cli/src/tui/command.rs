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
    /// Scheduled prompts: see, edit, enable/disable, run, cancel, delete.
    Triggers,
    /// `None` shows the current model; `Some` switches to it.
    Model(Option<String>),
    Provider(Option<String>),
    /// `None` shows the current permission mode; `Some` switches to it.
    Mode(Option<PermissionMode>),
    Usage,
    Clear,
    Session,
    /// Show or hide the live todo pane.
    Todo,
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
        // Both spellings: the command is `mecha trigger`, the thing you want
        // to see is all of them.
        "triggers" | "trigger" => Command::Triggers,
        "model" | "m" => Command::Model(arg.map(str::to_string)),
        "provider" | "p" => Command::Provider(arg.map(str::to_string)),
        "usage" => Command::Usage,
        "clear" | "new" => Command::Clear,
        "session" => Command::Session,
        "todo" => Command::Todo,
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

/// The `!command` shell escape: the command to run locally, or `None` if the
/// line is an ordinary message.
///
/// A bare `!` (or `!` followed by only whitespace) is not a command — same
/// rule as the bare `/`, and for the same reason: an exclamation someone
/// typed should not make the input line quietly lossy.
pub fn shell_escape(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix('!')?.trim();
    (!rest.is_empty()).then_some(rest)
}

/// The `@path` token containing the cursor, if there is one: the byte offset
/// where the partial path starts (just after the `@`) and the partial itself.
///
/// Cursor-relative, not line-relative, because the mention can sit anywhere
/// in a message — "summarise @docs/HAND" completes mid-sentence.
pub fn at_token(input: &str, cursor: usize) -> Option<(usize, &str)> {
    let cursor = cursor.min(input.len());
    let before = &input[..cursor];
    let token_start = before
        .rfind(|c: char| c.is_whitespace())
        .map(|i| i + before[i..].chars().next().map_or(1, char::len_utf8))
        .unwrap_or(0);
    let token = &before[token_start..];
    token
        .starts_with('@')
        .then(|| (token_start + 1, &token[1..]))
}

/// Workspace entries the partial path could still mean. Directories come with
/// a trailing `/`, so accepting one and pressing Tab again descends.
///
/// Dotfiles complete only when asked for by name, and `.git`/`target` only
/// when something is typed — completing into a build directory from an empty
/// partial is how a four-gigabyte listing happens.
pub fn path_candidates(partial: &str, workspace: &std::path::Path) -> Vec<String> {
    // Absolute and parent-escaping partials get nothing: completion serves
    // the workspace, and the UI should not teach paths the path jail will
    // refuse anyway.
    if partial.starts_with('/') || partial.split('/').any(|c| c == "..") {
        return Vec::new();
    }
    let (dir_part, file_part) = match partial.rfind('/') {
        Some(i) => (&partial[..=i], &partial[i + 1..]),
        None => ("", partial),
    };
    let Ok(entries) = std::fs::read_dir(workspace.join(dir_part)) else {
        return Vec::new();
    };

    let mut out: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with(file_part) {
                return None;
            }
            if name.starts_with('.') && !file_part.starts_with('.') {
                return None;
            }
            if (name == ".git" || name == "target") && file_part.is_empty() {
                return None;
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some(format!("{dir_part}{name}{}", if is_dir { "/" } else { "" }))
        })
        .collect();
    out.sort();
    // A cap, said out loud by the caller being unable to see past it — a
    // directory this wide needs a narrower partial, not a longer menu.
    out.truncate(200);
    out
}

/// Every command name, in the order they are offered for completion.
///
/// One list, so completion and `HELP` cannot drift apart — there is a test that
/// every name here parses, and another that everything `HELP` advertises is
/// here.
pub const NAMES: [&str; 11] = [
    "help", "tools", "triggers", "model", "provider", "mode", "mcp", "usage", "clear", "session",
    "todo",
];

/// Command names that could still be meant by what has been typed.
///
/// Empty for anything that is not a command being typed: once there is
/// whitespace the name is settled and the user is onto arguments, and once
/// there is an exact match there is nothing left to suggest.
pub fn completions(input: &str) -> Vec<&'static str> {
    let Some(rest) = input.strip_prefix('/') else {
        return Vec::new();
    };
    if rest.contains(char::is_whitespace) {
        return Vec::new();
    }
    let rest = rest.to_ascii_lowercase();
    NAMES
        .iter()
        .copied()
        .filter(|n| n.starts_with(&rest) && *n != rest)
        .collect()
}

/// The longest prefix every candidate shares — what Tab should fill in.
///
/// Completing to the *common* prefix rather than the first match is what makes
/// repeated Tab presses converge instead of cycling through guesses. Generic
/// so it serves both the `&'static str` command names and the owned path
/// candidates.
pub fn common_prefix<S: AsRef<str>>(candidates: &[S]) -> String {
    let Some(first) = candidates.first().map(AsRef::as_ref) else {
        return String::new();
    };
    let mut len = first.len();
    for c in &candidates[1..] {
        len = len.min(
            first
                .chars()
                .zip(c.as_ref().chars())
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
  /triggers              scheduled prompts: see, edit, run, cancel
  /model [id]            show or switch the model
  /provider [name]       show or switch the provider
  /mode [ask|allow|read-only]   show or switch the permission mode
  /mcp [on|off]          list MCP servers, or turn them all off and on
  /mcp <server> [on|off] turn one server off and on
  /usage                 tokens used this session
  /clear                 start a new conversation, dropping its taint
  /session               where the transcript is being written
  /todo                  show or hide the live task pane
  /exit                  quit";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_only_fires_while_the_name_is_still_being_typed() {
        assert_eq!(completions("/mo"), vec!["model", "mode"]);
        assert_eq!(completions("/mod"), vec!["model", "mode"]);
        assert_eq!(
            completions("/mode"),
            vec!["model"],
            "an exact match still offers longer names"
        );
        assert_eq!(completions("/c"), vec!["clear"]);

        // Not a command, or past the name: nothing to suggest.
        assert!(completions("summarise this").is_empty());
        assert!(
            completions("/model claude").is_empty(),
            "arguments are not command names"
        );
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
        assert_eq!(common_prefix::<&str>(&[]), "");
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
            let Some(advertised) = line.split_whitespace().next() else {
                continue;
            };
            let advertised = advertised.trim_start_matches('/');
            if advertised == "exit" {
                continue; // an alias, deliberately not offered first
            }
            assert!(
                NAMES.contains(&advertised),
                "{advertised} is documented but not completable"
            );
        }
    }

    #[test]
    fn an_ordinary_message_is_not_a_command() {
        assert_eq!(parse("summarise the README"), None);
        assert_eq!(parse("what does a/b mean?"), None);
    }

    /// A throwaway directory tree for the path-completion tests.
    fn fixture() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!(
            "mecha-completion-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        for dir in ["docs", "src", "target", ".git"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in [
            "README.md",
            "docs/HANDOFF.md",
            "docs/TUI-RESEARCH.md",
            ".hidden",
        ] {
            std::fs::write(root.join(file), "x").unwrap();
        }
        root
    }

    #[test]
    fn the_at_token_is_found_anywhere_in_the_message() {
        // Mentions complete mid-sentence, not only at the start of the line.
        assert_eq!(at_token("@doc", 4), Some((1, "doc")));
        assert_eq!(
            at_token("summarise @docs/HAND", 20),
            Some((11, "docs/HAND"))
        );
        assert_eq!(at_token("read @", 6), Some((6, "")));

        // No @-token at the cursor: nothing to complete.
        assert_eq!(at_token("plain text", 10), None);
        assert_eq!(
            at_token("a@b.com is an email", 5),
            None,
            "@ mid-word is not a mention"
        );
        assert_eq!(at_token("@docs done", 10), None, "cursor is past the token");
    }

    #[test]
    fn path_candidates_complete_the_workspace_and_descend_directories() {
        let root = fixture();

        let top = path_candidates("", &root);
        assert!(top.contains(&"docs/".to_string()), "{top:?}");
        assert!(top.contains(&"README.md".to_string()), "{top:?}");

        // Directories carry the trailing slash, so the next Tab descends.
        assert_eq!(path_candidates("do", &root), vec!["docs/"]);
        let inside = path_candidates("docs/", &root);
        assert_eq!(inside, vec!["docs/HANDOFF.md", "docs/TUI-RESEARCH.md"]);
        assert_eq!(
            common_prefix(&inside),
            "docs/",
            "diverging names share only the dir"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn noise_and_escapes_are_not_offered() {
        let root = fixture();

        let top = path_candidates("", &root);
        assert!(!top.iter().any(|c| c.contains(".git")), "{top:?}");
        assert!(!top.iter().any(|c| c.contains("target")), "{top:?}");
        assert!(!top.iter().any(|c| c.contains(".hidden")), "{top:?}");

        // Asked for by name, dotfiles and the build dir do complete.
        assert_eq!(path_candidates(".hi", &root), vec![".hidden"]);
        assert_eq!(path_candidates("targ", &root), vec!["target/"]);

        // The jail will refuse these, so the UI does not teach them.
        assert!(path_candidates("/etc/pass", &root).is_empty());
        assert!(path_candidates("../up", &root).is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_shell_escape_needs_a_command_after_the_bang() {
        assert_eq!(shell_escape("!ls -la"), Some("ls -la"));
        assert_eq!(shell_escape("  !git status  "), Some("git status"));

        // A lone `!` is punctuation someone typed, not a command.
        assert_eq!(shell_escape("!"), None);
        assert_eq!(shell_escape("!   "), None);
        assert_eq!(shell_escape("fix it!"), None);
        assert_eq!(shell_escape("hello"), None);
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
        assert_eq!(
            parse("/model claude-opus-5"),
            Some(Command::Model(Some("claude-opus-5".into())))
        );
        assert_eq!(
            parse("/provider local"),
            Some(Command::Provider(Some("local".into())))
        );
        assert_eq!(parse("/clear"), Some(Command::Clear));
    }

    #[test]
    fn surrounding_whitespace_never_decides_anything() {
        assert_eq!(parse("  /usage  "), Some(Command::Usage));
        assert_eq!(
            parse("/model    gpt-4o   "),
            Some(Command::Model(Some("gpt-4o".into())))
        );
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
            assert_eq!(
                parse(&format!("/mode {word}")),
                Some(Command::Mode(Some(PermissionMode::Allow)))
            );
        }
        for word in ["read-only", "readonly", "ro", "plan"] {
            assert_eq!(
                parse(&format!("/mode {word}")),
                Some(Command::Mode(Some(PermissionMode::ReadOnly))),
                "{word}"
            );
        }
        assert_eq!(
            parse("/mode ask"),
            Some(Command::Mode(Some(PermissionMode::Ask)))
        );
        assert_eq!(parse("/mode"), Some(Command::Mode(None)));
    }

    #[test]
    fn mcp_toggles_on_the_words_people_actually_type() {
        for word in ["on", "yes", "true", "enable", "ON"] {
            assert_eq!(
                parse(&format!("/mcp {word}")),
                Some(Command::Mcp(Some(true))),
                "{word}"
            );
        }
        for word in ["off", "no", "false", "disable"] {
            assert_eq!(
                parse(&format!("/mcp {word}")),
                Some(Command::Mcp(Some(false))),
                "{word}"
            );
        }
        assert_eq!(parse("/mcp"), Some(Command::Mcp(None)));
        // A single word that is not on/off is a server name — the parser
        // cannot know which servers exist, so an unknown one is caught at
        // dispatch, where the configured list can be shown.
        assert_eq!(
            parse("/mcp maybe"),
            Some(Command::McpServer("maybe".into(), None))
        );
    }

    #[test]
    fn mcp_addresses_all_the_servers_or_one_of_them() {
        assert_eq!(parse("/mcp off"), Some(Command::Mcp(Some(false))));
        assert_eq!(
            parse("/mcp pkg off"),
            Some(Command::McpServer("pkg".into(), Some(false)))
        );
        assert_eq!(
            parse("/mcp pkg on"),
            Some(Command::McpServer("pkg".into(), Some(true)))
        );
        // A bare name flips it, which is what you want when there is one
        // server you keep reaching for.
        assert_eq!(
            parse("/mcp pkg"),
            Some(Command::McpServer("pkg".into(), None))
        );

        // Reads naturally, means the opposite of what it looks like. Refused
        // rather than guessed at.
        assert_eq!(
            parse("/mcp off pkg"),
            Some(Command::BadToggle("off pkg".into()))
        );
        assert_eq!(
            parse("/mcp pkg maybe"),
            Some(Command::BadToggle("maybe".into()))
        );
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
            let Some(name) = line.split_whitespace().next() else {
                continue;
            };
            let parsed = parse(name);
            assert!(
                !matches!(parsed, None | Some(Command::Unknown(_))),
                "{name} is advertised but does not parse"
            );
        }
    }
}
