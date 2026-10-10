//! The guided `mecha setup` (ruling F14, `docs/FEATURES-DESIGN.md` §11): one
//! pass, never a detour. The owner, 2026-10-10: *"I don't want to have to
//! stop setup to run setup chat explicitly, that makes setup more confusing
//! and less easy."*
//!
//! Chat first, because nothing below it can be tested without it; then each
//! feature that is off as a plain yes or no, with what it does and what it
//! downloads; then every answer at once — one total, one yes — and the
//! installs. What is left (the charter, the scheduler, anything switched on
//! but not yet working) goes to `setup`'s own step loop afterwards.
//!
//! The questions are functions of a reader, so the answers a person can give
//! are tested without a terminal; the installs are `setup chat`'s and
//! `features enable`'s own, given the one yes rather than asking again.

use anyhow::Result;
use mecha_core::feature::{self, Feature};
use mecha_core::onboarding::{self, Facts, Status, Step};
use mecha_core::router_unit::Choice;
use std::io::{BufRead, Write};

/// What the owner chose for the chat model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ChatPick {
    /// Something already answers prompts: nothing to do.
    Answering,
    /// The model recommended for this machine.
    Recommended,
    /// A GGUF the owner has.
    Own {
        model: std::path::PathBuf,
        mmproj: Option<std::path::PathBuf>,
    },
    /// A hosted provider instead: Anthropic, with its key from the
    /// environment.
    Hosted,
    /// A server answers but the config does not say what it serves: write
    /// what it reports.
    WriteServer,
    /// Not now.
    Skip,
}

/// What the chat question may offer on this machine.
#[derive(Debug, Clone, Default)]
pub(super) struct ChatOptions {
    /// The recommended model and its download, when one can be installed here.
    pub recommended: Option<(String, u64)>,
    /// Whether `setup chat` can install a router here at all — Linux, and
    /// nobody else's server on the port.
    pub can_install: bool,
    /// A server answers and the config disagrees with it.
    pub server_disagrees: bool,
    /// `ANTHROPIC_API_KEY` is set.
    pub hosted_key: bool,
}

/// One line read and trimmed; end of input reads as an empty answer.
fn line(read: &mut impl BufRead) -> Result<String> {
    Ok(answer(read)?.unwrap_or_default())
}

/// One line read and trimmed, or `None` at the end of input — which is never
/// the same as Enter: Enter takes a default, the end of input takes nothing.
fn answer(read: &mut impl BufRead) -> Result<Option<String>> {
    let mut l = String::new();
    Ok((read.read_line(&mut l)? > 0).then(|| l.trim().to_string()))
}

fn prompt(text: &str) -> Result<()> {
    print!("{text}");
    std::io::stdout().flush()?;
    Ok(())
}

fn gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0)
}

/// `~` and `~/…` as a shell would expand them; anything else as typed.
fn expand_home(typed: &str) -> std::path::PathBuf {
    match (typed, dirs::home_dir()) {
        ("~", Some(h)) => h,
        (t, Some(h)) if t.starts_with("~/") => h.join(&t[2..]),
        (t, _) => t.into(),
    }
}

/// The chat question. The menu holds only what this machine can do: no
/// install offered where `setup chat` would refuse, no recommended model
/// where none fits.
pub(super) fn ask_chat(read: &mut impl BufRead, o: &ChatOptions) -> Result<ChatPick> {
    println!("\nChat model");
    if o.server_disagrees {
        println!(
            "  A server answers here, but the config does not say what it serves. Setup can \
             write what the server reports about itself."
        );
        prompt("  Write it? [Y/n] ")?;
        // End of input is not Enter: nothing is written by a Ctrl-D (found
        // on review of #631).
        return Ok(
            match answer(read)?.map(|a| a.to_ascii_lowercase()).as_deref() {
                Some("" | "y" | "yes") => ChatPick::WriteServer,
                None => {
                    println!();
                    ChatPick::Skip
                }
                _ => ChatPick::Skip,
            },
        );
    }
    let mut menu: Vec<(String, ChatPick)> = Vec::new();
    if o.can_install {
        if let Some((model, bytes)) = &o.recommended {
            menu.push((
                format!(
                    "Use the recommended model: {model} ({} download)",
                    gib(*bytes)
                ),
                ChatPick::Recommended,
            ));
        }
        menu.push((
            "Use a GGUF I already have".into(),
            ChatPick::Own {
                model: Default::default(),
                mmproj: None,
            },
        ));
    } else {
        println!(
            "  Nothing answers prompts yet, and setup cannot install a local model here — the \
             line above says why, or the config's local provider names a server elsewhere. \
             Start that server and run `mecha setup --write`, or use a hosted model."
        );
    }
    menu.push((
        format!(
            "Use a hosted model instead (Anthropic{})",
            if o.hosted_key {
                " — your ANTHROPIC_API_KEY is set"
            } else {
                " — needs ANTHROPIC_API_KEY"
            }
        ),
        ChatPick::Hosted,
    ));
    menu.push(("Skip for now".into(), ChatPick::Skip));
    for (i, (text, _)) in menu.iter().enumerate() {
        println!("  {}) {text}", i + 1);
    }
    loop {
        prompt("  Which? [1] ")?;
        // End of input is a skip, never a loop that cannot end.
        let Some(answer) = answer(read)? else {
            println!();
            return Ok(ChatPick::Skip);
        };
        let pick = if answer.is_empty() {
            Some(0)
        } else {
            answer.parse::<usize>().ok().and_then(|n| n.checked_sub(1))
        };
        match pick.and_then(|i| menu.get(i)) {
            Some((_, ChatPick::Own { .. })) => return ask_own(read),
            Some((_, pick)) => return Ok(pick.clone()),
            None => println!("  `{answer}` is not one of the choices"),
        }
    }
}

/// The owner's own GGUF: a path that is a file, and its projector if any.
fn ask_own(read: &mut impl BufRead) -> Result<ChatPick> {
    loop {
        prompt("  Path to the model's GGUF (Enter to skip): ")?;
        let typed = line(read)?;
        if typed.is_empty() {
            return Ok(ChatPick::Skip);
        }
        let model = expand_home(&typed);
        if !model.is_file() {
            println!("  {} is not a file", model.display());
            continue;
        }
        prompt("  Its vision projector (mmproj), or Enter for none: ")?;
        let typed = line(read)?;
        let mmproj = (!typed.is_empty()).then(|| expand_home(&typed));
        return Ok(ChatPick::Own { model, mmproj });
    }
}

/// A feature to ask about: one that is off, unanswered, and has a switch.
#[derive(Debug, Clone)]
pub(super) struct Question {
    pub feature: Feature,
    /// What `features enable` would download for it, when it installs
    /// anything here; `None` when it installs nothing.
    pub bytes: Option<u64>,
    /// The part of `bytes` that is llama.cpp — shared by every plan that
    /// names it, so counted once in the total.
    pub engine: u64,
}

/// The features `setup` would offer one by one — the optional, unanswered
/// ones — in the order setup lists them.
pub(super) fn feature_questions(steps: &[Step]) -> Vec<Feature> {
    steps
        .iter()
        .filter(|s| s.optional && s.status == Status::Missing)
        .filter_map(|s| Feature::parse(&s.id))
        .filter(|f| f.has_switch())
        .collect()
}

/// Each feature as a yes or no, with what it is and what it downloads.
pub(super) fn ask_features(read: &mut impl BufRead, qs: &[Question]) -> Result<Vec<Feature>> {
    println!("\nFeatures — enable each? (all can be changed later with `mecha features`)");
    let mut yes = Vec::new();
    for q in qs {
        let cost = match q.bytes {
            Some(b) if b > 0 => format!(" ({} download)", gib(b)),
            _ => String::new(),
        };
        // A yes switches on what the feature needs too, so it is said here
        // rather than discovered (found on review of #631).
        let needs: Vec<&str> = q.feature.requires().iter().map(|r| r.label()).collect();
        let needs = if needs.is_empty() {
            String::new()
        } else {
            format!(" — needs {} too", needs.join(", ").to_lowercase())
        };
        println!("  {}{cost}{needs}", q.feature.label());
        let blurb = onboarding::blurb(q.feature);
        if !blurb.is_empty() {
            println!("    {blurb}");
        }
        prompt("    Enable? [y/N] ")?;
        if matches!(line(read)?.to_ascii_lowercase().as_str(), "y" | "yes") {
            yes.push(q.feature);
        }
    }
    Ok(yes)
}

/// What is about to happen, said before the one yes that starts it.
pub(super) fn summary(
    chat: &ChatPick,
    chat_bytes: u64,
    features: &[Feature],
    total: u64,
) -> String {
    let mut out = String::from("\nReady:\n");
    match chat {
        ChatPick::Recommended => out.push_str(&format!(
            "  the recommended chat model and the router that serves it ({})\n",
            gib(chat_bytes)
        )),
        ChatPick::Own { model, .. } => out.push_str(&format!(
            "  the router, serving {}{}\n",
            model.display(),
            if chat_bytes > 0 {
                format!(" (llama.cpp, {})", gib(chat_bytes))
            } else {
                String::new()
            }
        )),
        ChatPick::Hosted => out.push_str("  Anthropic as the default provider\n"),
        ChatPick::WriteServer => out.push_str("  the running server's settings into the config\n"),
        ChatPick::Answering | ChatPick::Skip => {}
    }
    for f in features {
        out.push_str(&format!("  {}\n", f.label()));
    }
    if total > 0 {
        out.push_str(&format!(
            "{} to download, less whatever is already in the cache.\n",
            gib(total)
        ));
    }
    out
}

/// Start? — the one confirmation, defaulting to yes since every line above
/// it was already answered.
pub(super) fn ask_start(read: &mut impl BufRead) -> Result<bool> {
    prompt("Start? [Y/n] ")?;
    // End of input is not Enter: nothing is started by a Ctrl-D (found on
    // review of #631) — a Ctrl-D at the one confirmation began the downloads.
    let Some(answer) = answer(read)? else {
        println!();
        return Ok(false);
    };
    Ok(matches!(
        answer.to_ascii_lowercase().as_str(),
        "" | "y" | "yes"
    ))
}

/// The steps the chat question answers.
pub(super) const CHAT_STEPS: [&str; 2] = ["local-server", "provider-credential"];

/// Whether something can already answer a prompt: neither of the steps that
/// say whether one can is outstanding.
fn chat_answering(steps: &[Step]) -> bool {
    !steps
        .iter()
        .any(|s| CHAT_STEPS.contains(&s.id.as_str()) && s.status != Status::Done)
}

/// The guided pass. Returns the ids of the steps it dealt with, so `setup`'s
/// own loop asks only about what is left.
pub(super) async fn run(
    cfg: &mecha_core::config::Config,
    provider_name: &str,
    steps: &[Step],
    facts: &Facts,
    home: &std::path::Path,
    read: &mut impl BufRead,
) -> Result<Vec<String>> {
    // The chat steps join `handled` only once the answer settled them, so a
    // skip, a "no" to Start, or a failed install leaves them on the
    // checklist `setup` prints after this (review of #631).
    let mut handled: Vec<String> = Vec::new();
    let settle_chat = |handled: &mut Vec<String>| {
        handled.extend(CHAT_STEPS.iter().map(|s| s.to_string()));
    };

    // --- the chat question, and what answering it would cost
    let server_disagrees = steps
        .iter()
        .any(|s| s.id == "local-server" && s.status == Status::Wrong);
    // Whether `setup chat` could install here is asked only when nothing
    // answers prompts and no server is merely misdescribed — a hosted
    // provider missing its key included, so the local route is offered there
    // too.
    // Nor for a local provider that names a server elsewhere: `setup chat`
    // leaves it as it is, so installing the router would not answer for it
    // (the same rule as the `local-server` step's remedy, review of #627).
    let elsewhere = cfg.providers.get(provider_name).is_some_and(|p| {
        p.kind == "local"
            && !mecha_core::router_unit::names_this_router(
                p,
                mecha_core::router_unit::Naming::shipped().port,
            )
    });
    let ready =
        if !chat_answering(steps) && !server_disagrees && !elsewhere && cfg!(target_os = "linux") {
            // Prints its own reason when it will not install here.
            match super::setup_chat::prepare().await {
                Ok(r) => r,
                Err(e) => {
                    println!("  {e:#}");
                    None
                }
            }
        } else {
            None
        };
    let chat = if chat_answering(steps) {
        if let Some(s) = steps.iter().find(|s| s.id == "local-server") {
            println!("\n✓ Chat: {}", s.detail);
        }
        ChatPick::Answering
    } else {
        let options = ChatOptions {
            recommended: ready.as_ref().and_then(|r| {
                r.row
                    .map(|row| (row.model.to_string(), r.row_bytes().unwrap_or(0)))
            }),
            can_install: ready.is_some(),
            server_disagrees,
            hosted_key: std::env::var_os("ANTHROPIC_API_KEY").is_some(),
        };
        ask_chat(read, &options)?
    };
    let engine_for_chat = match (&chat, &ready) {
        (ChatPick::Recommended | ChatPick::Own { .. }, Some(r)) => r.engine_bytes.unwrap_or(0),
        _ => 0,
    };
    let chat_bytes = match (&chat, &ready) {
        (ChatPick::Recommended, Some(r)) => r.row_bytes().unwrap_or(0) + engine_for_chat,
        (ChatPick::Own { .. }, Some(_)) => engine_for_chat,
        _ => 0,
    };

    // --- the features, each with what it would download here
    let asked = feature_questions(steps);
    // A machine that could not be read prices nothing, and asks anyway:
    // setup must not fail before its first question over a size.
    let questions = if asked.is_empty() {
        Vec::new()
    } else {
        price(cfg, &asked).unwrap_or_else(|e| {
            println!("\n(sizes unavailable: {e:#})");
            asked
                .iter()
                .map(|f| Question {
                    feature: *f,
                    bytes: None,
                    engine: 0,
                })
                .collect()
        })
    };
    let chosen = if questions.is_empty() {
        Vec::new()
    } else {
        ask_features(read, &questions)?
    };
    handled.extend(asked.iter().map(|f| f.id().to_string()));

    // --- one total, one yes
    let total = chat_bytes + features_total(&questions, &chosen, engine_for_chat > 0);
    let nothing = matches!(chat, ChatPick::Answering | ChatPick::Skip) && chosen.is_empty();
    if nothing {
        return Ok(handled);
    }
    print!("{}", summary(&chat, chat_bytes, &chosen, total));
    if !ask_start(read)? {
        println!("Nothing was changed. `mecha setup` asks again.");
        return Ok(handled);
    }

    // --- the installs, in the order they were asked
    match &chat {
        ChatPick::Recommended | ChatPick::Own { .. } => {
            if let Some(ready) = &ready {
                let choice = match &chat {
                    ChatPick::Own { model, mmproj } => Choice::Own {
                        model: model.clone(),
                        mmproj: mmproj.clone(),
                    },
                    _ => Choice::Recommended,
                };
                println!("\nInstalling the chat model…");
                match super::setup_chat::install_choice(cfg, ready, &choice, true).await {
                    Ok(()) => settle_chat(&mut handled),
                    Err(e) => {
                        println!("the chat model was not installed: {e:#}");
                        println!("`mecha setup chat` tries it again on its own.");
                    }
                }
            }
        }
        ChatPick::Hosted => {
            super::setup::offer_default("anthropic", &cfg.default_provider, true)?;
            settle_chat(&mut handled);
            if std::env::var_os("ANTHROPIC_API_KEY").is_none() {
                println!(
                    "Set ANTHROPIC_API_KEY in your shell (`export ANTHROPIC_API_KEY=…`) and start \
                     a new one — mecha keeps the variable's name, never the key."
                );
            }
        }
        ChatPick::WriteServer => {
            if let Some(props) = &facts.props {
                if super::setup::offer_settings(provider_name, props, true)? {
                    settle_chat(&mut handled);
                }
            }
        }
        ChatPick::Answering | ChatPick::Skip => {}
    }
    // One feature at a time, each with what it needs: a batch is
    // all-or-nothing in `plan_enable`, so one yes whose requirement was
    // answered no cost every other yes (found on review of #631).
    let mut enabled = Vec::new();
    for f in &chosen {
        let now = mecha_core::config::Config::load_global()?;
        let ids = enable_ids(&feature::enable_command(&now, *f));
        if ids.is_empty() {
            continue;
        }
        println!("\nEnabling {}…", ids.join(", "));
        match super::features::enable(&ids, false, true).await {
            Ok(()) => enabled.push(*f),
            Err(e) => println!("`{}` not enabled: {e:#} — `mecha setup` asks again", f.id()),
        }
    }
    if !enabled.is_empty() {
        sign_ins(&enabled, home)?;
    }
    Ok(handled)
}

/// The ids `feature::enable_command` names — the feature and every switch it
/// hangs on that is not on yet, dependencies first.
pub(super) fn enable_ids(command: &str) -> Vec<String> {
    command
        .split_whitespace()
        .skip_while(|w| *w != "enable")
        .skip(1)
        .map(str::to_string)
        .collect()
}

/// What each chosen feature would download here — through `enable`'s own
/// plan, so the figure asked about is the figure fetched.
fn price(cfg: &mecha_core::config::Config, features: &[Feature]) -> Result<Vec<Question>> {
    let chat_here = mecha_core::install::chat_runs_here(cfg);
    // The machine is read only when some question would install something.
    if !features
        .iter()
        .any(|f| mecha_core::install::may_offer(*f, chat_here))
    {
        return Ok(features
            .iter()
            .map(|f| Question {
                feature: *f,
                bytes: None,
                engine: 0,
            })
            .collect());
    }
    let m = mecha_core::sidecar::Machinery::real()?;
    let machine = mecha_core::recommend::Machine::read()?;
    let hub = mecha_core::fetch::hub_dir()?;
    let nvidia = std::cell::OnceCell::new();
    let read_nvidia = || *nvidia.get_or_init(mecha_core::engine::read_nvidia);
    let mut out = Vec::new();
    for f in features {
        let (bytes, engine) = if mecha_core::install::may_offer(*f, chat_here) {
            let mut p = mecha_core::sidecar::plan(*f, &m, &machine, &hub, false)?;
            mecha_core::install::price(&mut p, chat_here, read_nvidia);
            let engine = p
                .sidecars
                .iter()
                .find(|s| s.id == "llama")
                .and_then(|s| s.bytes)
                .unwrap_or(0);
            (Some(p.download_bytes), engine)
        } else {
            (None, 0)
        };
        out.push(Question {
            feature: *f,
            bytes,
            engine,
        });
    }
    Ok(out)
}

/// The chosen features' downloads, with llama.cpp counted once however many
/// plans name it — and not at all when the chat install already brings it.
pub(super) fn features_total(qs: &[Question], chosen: &[Feature], engine_counted: bool) -> u64 {
    let picked = qs.iter().filter(|q| chosen.contains(&q.feature));
    let own: u64 = picked
        .clone()
        .map(|q| q.bytes.unwrap_or(0).saturating_sub(q.engine))
        .sum();
    let engine = if engine_counted {
        0
    } else {
        picked.map(|q| q.engine).max().unwrap_or(0)
    };
    own + engine
}

/// A feature switched on but not ready only for want of an account is signed
/// in to now, while the owner is here (F14 item 2): its own next command,
/// run at this terminal.
fn sign_ins(chosen: &[Feature], home: &std::path::Path) -> Result<()> {
    let cfg = mecha_core::config::Config::load_global()?;
    let rows = feature::all(&feature::Facts::read(home, &cfg));
    for f in chosen {
        let Some(row) = rows.iter().find(|r| r.id == *f) else {
            continue;
        };
        match (&row.state, row.next.as_deref()) {
            (feature::State::On { .. }, _) => println!("`{}` is on", f.id()),
            (feature::State::Unready { reason, .. }, Some(next)) => {
                println!("\n`{}` needs one more thing: {reason}.", f.id());
                match onboarding::runnable(next) {
                    Some((argv, _)) if !argv.iter().any(|a| a == "enable") => {
                        println!("Running `{}`…", argv.join(" "));
                        let status = std::process::Command::new(&argv[0])
                            .args(&argv[1..])
                            .status();
                        if !matches!(status, Ok(s) if s.success()) {
                            println!("that did not finish — `mecha setup` asks again");
                        }
                    }
                    _ => println!("Next: {next}"),
                }
            }
            (state, next) => println!(
                "`{}` reads {}{}",
                f.id(),
                state.word(),
                next.map(|n| format!(" — next: {n}")).unwrap_or_default()
            ),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader(s: &str) -> std::io::Cursor<Vec<u8>> {
        std::io::Cursor::new(s.as_bytes().to_vec())
    }

    fn options() -> ChatOptions {
        ChatOptions {
            recommended: Some(("Qwen-x".into(), 22 << 30)),
            can_install: true,
            server_disagrees: false,
            hosted_key: false,
        }
    }

    /// Enter takes the recommended model; each number is its line; a hosted
    /// model and skipping are always there.
    #[test]
    fn the_chat_menu_reads_each_answer() {
        assert_eq!(
            ask_chat(&mut reader("\n"), &options()).unwrap(),
            ChatPick::Recommended
        );
        assert_eq!(
            ask_chat(&mut reader("1\n"), &options()).unwrap(),
            ChatPick::Recommended
        );
        assert_eq!(
            ask_chat(&mut reader("3\n"), &options()).unwrap(),
            ChatPick::Hosted
        );
        assert_eq!(
            ask_chat(&mut reader("4\n"), &options()).unwrap(),
            ChatPick::Skip
        );
        // A wrong answer asks again; end of input skips rather than loops.
        assert_eq!(
            ask_chat(&mut reader("9\n3\n"), &options()).unwrap(),
            ChatPick::Hosted
        );
        assert_eq!(
            ask_chat(&mut reader("9\n"), &options()).unwrap(),
            ChatPick::Skip
        );
    }

    /// Where nothing can be installed, the menu offers only what can be
    /// done: the numbering is of what is shown.
    #[test]
    fn the_menu_offers_no_install_where_none_can_run() {
        let o = ChatOptions {
            can_install: false,
            ..options()
        };
        assert_eq!(ask_chat(&mut reader("1\n"), &o).unwrap(), ChatPick::Hosted);
        assert_eq!(ask_chat(&mut reader("2\n"), &o).unwrap(), ChatPick::Skip);
        // No recommended row: the GGUF is first.
        let o = ChatOptions {
            recommended: None,
            ..options()
        };
        assert_eq!(ask_chat(&mut reader("1\n\n"), &o).unwrap(), ChatPick::Skip);
    }

    /// A GGUF must be a file; asked again until it is, Enter to give up.
    #[test]
    fn an_own_gguf_is_a_file() {
        let dir = std::env::temp_dir().join(format!("mecha-f14-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let gguf = dir.join("m.gguf");
        std::fs::write(&gguf, b"").unwrap();
        let answers = format!("2\n{}/missing.gguf\n{}\n\n", dir.display(), gguf.display());
        let got = ask_chat(&mut reader(&answers), &options()).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            got,
            ChatPick::Own {
                model: gguf.clone(),
                mmproj: None
            }
        );
    }

    /// A server that answers but disagrees is one question, defaulting to
    /// writing what it reports.
    #[test]
    fn a_disagreeing_server_is_asked_to_be_written_down() {
        let o = ChatOptions {
            server_disagrees: true,
            ..options()
        };
        assert_eq!(
            ask_chat(&mut reader("\n"), &o).unwrap(),
            ChatPick::WriteServer
        );
        assert_eq!(ask_chat(&mut reader("n\n"), &o).unwrap(), ChatPick::Skip);
        // End of input writes nothing (review of #631).
        assert_eq!(ask_chat(&mut reader(""), &o).unwrap(), ChatPick::Skip);
    }

    /// Only optional, unanswered features with a switch are asked about.
    #[test]
    fn only_unanswered_features_are_asked() {
        let step = |id: &str, status: Status, optional: bool| Step {
            id: id.into(),
            title: id.into(),
            status,
            detail: String::new(),
            remedy: None,
            optional,
            undo: None,
        };
        let steps = vec![
            step("config-file", Status::Missing, false),
            step("local-server", Status::Missing, false),
            step("web", Status::Missing, true),
            step("mail", Status::Done, true),
            step("slack", Status::Declined, true),
            step("documents", Status::Missing, true),
            step("charter", Status::Missing, true),
        ];
        assert_eq!(
            feature_questions(&steps),
            vec![Feature::Web, Feature::Documents]
        );
        assert!(!chat_answering(&steps));
    }

    /// Each feature's answer is its own; anything but yes is no.
    #[test]
    fn each_feature_is_a_yes_or_no() {
        let qs = [
            Question {
                feature: Feature::Web,
                bytes: None,
                engine: 0,
            },
            Question {
                feature: Feature::Documents,
                bytes: Some(1 << 30),
                engine: 0,
            },
            Question {
                feature: Feature::Search,
                bytes: None,
                engine: 0,
            },
        ];
        assert_eq!(
            ask_features(&mut reader("y\n\nyes\n"), &qs).unwrap(),
            vec![Feature::Web, Feature::Search]
        );
    }

    /// llama.cpp is in every plan that runs a server on it, and downloaded
    /// once: counted once, and not at all when the chat install brings it.
    #[test]
    fn the_engine_is_counted_once() {
        let q = |f, own: u64| Question {
            feature: f,
            bytes: Some(own + 700),
            engine: 700,
        };
        let qs = [q(Feature::Documents, 100), q(Feature::Graph, 50)];
        let both = [Feature::Documents, Feature::Graph];
        assert_eq!(features_total(&qs, &both, false), 100 + 50 + 700);
        assert_eq!(features_total(&qs, &both, true), 100 + 50);
        assert_eq!(features_total(&qs, &[Feature::Graph], false), 50 + 700);
        assert_eq!(features_total(&qs, &[], false), 0);
    }

    /// A yes enables the feature with what it needs, parsed from the same
    /// command the step loop runs — so `incognito` brings `web` (review of
    /// #631).
    #[test]
    fn a_yes_brings_what_the_feature_needs() {
        let cfg = mecha_core::config::Config::default();
        let ids = enable_ids(&feature::enable_command(&cfg, Feature::Incognito));
        assert_eq!(ids, ["web", "incognito"]);
        assert_eq!(
            enable_ids(&feature::enable_command(&cfg, Feature::Search)),
            ["search"]
        );
        assert!(enable_ids("nothing to run").is_empty());
    }

    /// The summary names what will happen and the one total.
    #[test]
    fn the_summary_names_each_install_and_the_total() {
        let s = summary(
            &ChatPick::Recommended,
            22 << 30,
            &[Feature::Web, Feature::Documents],
            23 << 30,
        );
        assert!(s.contains("recommended chat model"), "{s}");
        assert!(
            s.contains("The web app") && s.contains("PDF extraction"),
            "{s}"
        );
        assert!(s.contains("23.0 GiB to download"), "{s}");
        assert!(ask_start(&mut reader("\n")).unwrap());
        assert!(!ask_start(&mut reader("n\n")).unwrap());
        // End of input starts nothing (review of #631).
        assert!(!ask_start(&mut reader("")).unwrap());
    }
}
