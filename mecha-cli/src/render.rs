//! Terminal rendering of agent events.
//!
//! Streams the answer as it arrives and narrates tool use around it. Colour is
//! used only when stdout is a terminal, so piped output stays clean.

use mecha_core::agent::AgentEvent;
use mecha_core::message::Usage;
use std::io::{IsTerminal, Write};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;

#[derive(Clone, Copy, Default)]
pub struct RenderOpts {
    /// Show thinking, tool arguments, tool output, and per-turn usage.
    pub verbose: bool,
    /// Suppress everything except the final answer text.
    pub quiet: bool,
}

struct Style {
    on: bool,
}

impl Style {
    fn new() -> Self {
        // NO_COLOR is the de-facto standard opt-out.
        Style {
            on: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }

    fn dim(&self, s: &str) -> String {
        if self.on {
            format!("\x1b[2m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }

    fn cyan(&self, s: &str) -> String {
        if self.on {
            format!("\x1b[36m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }

    fn red(&self, s: &str) -> String {
        if self.on {
            format!("\x1b[31m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
}

/// Drain `rx` on a background task, printing as events arrive.
pub fn spawn(mut rx: UnboundedReceiver<AgentEvent>, opts: RenderOpts) -> JoinHandle<()> {
    tokio::spawn(async move {
        let style = Style::new();
        let mut out = std::io::stdout();
        // Tool narration has to start on its own line, but only if the model
        // was mid-sentence when it called the tool.
        let mut mid_line = false;

        while let Some(event) = rx.recv().await {
            match event {
                AgentEvent::TextDelta(t) => {
                    print!("{t}");
                    mid_line = !t.ends_with('\n');
                    let _ = out.flush();
                }

                AgentEvent::ThinkingDelta(t) if opts.verbose => {
                    print!("{}", style.dim(&t));
                    mid_line = !t.ends_with('\n');
                    let _ = out.flush();
                }

                AgentEvent::ToolCall { name, input, .. } if !opts.quiet => {
                    if mid_line {
                        println!();
                        mid_line = false;
                    }
                    let detail = if opts.verbose {
                        serde_json::to_string(&input).unwrap_or_default()
                    } else {
                        one_line(&input)
                    };
                    println!("{} {} {}", style.cyan("→"), style.cyan(&name), style.dim(&detail));
                    let _ = out.flush();
                }

                AgentEvent::ToolResult { name, is_error, content, .. } if !opts.quiet => {
                    if is_error {
                        println!("{} {}", style.red("✗"), style.red(&first_line(&content)));
                    } else if opts.verbose {
                        println!("{}", style.dim(&indent(&truncate(&content, 2_000))));
                    } else {
                        println!(
                            "{} {}",
                            style.dim("✓"),
                            style.dim(&format!("{name} — {}", size_hint(&content)))
                        );
                    }
                    let _ = out.flush();
                }

                AgentEvent::ToolDenied { name, reason } if !opts.quiet => {
                    println!("{} {}", style.red(&format!("✗ {name}")), style.dim(&reason));
                }

                AgentEvent::TurnUsage(usage) if opts.verbose => {
                    println!("{}", style.dim(&format!("  {}", format_usage(&usage))));
                }

                AgentEvent::Done(outcome) => {
                    if mid_line {
                        println!();
                        mid_line = false;
                    }
                    if let Some(refusal) = &outcome.refusal {
                        eprintln!(
                            "{}",
                            style.red(&format!(
                                "refused ({}): {}",
                                refusal.category.as_deref().unwrap_or("unspecified"),
                                refusal.explanation.as_deref().unwrap_or("no explanation given")
                            ))
                        );
                    }
                    if outcome.exhausted {
                        let fix = match outcome.stop_cause {
                            mecha_core::agent::StopCause::MaxTurns => "raise --max-turns",
                            mecha_core::agent::StopCause::OutputTokenBudget => {
                                "raise --max-output-tokens"
                            }
                            mecha_core::agent::StopCause::CostBudget => "raise --max-cost",
                            mecha_core::agent::StopCause::Completed => "",
                        };
                        eprintln!(
                            "{}",
                            style.red(&format!(
                                "{} after {} turns — the answer may be incomplete ({fix})",
                                outcome.stop_cause.describe(),
                                outcome.turns
                            ))
                        );
                    }
                    if opts.verbose {
                        let cost = outcome
                            .cost_usd
                            .map(|c| format!(" · ${c:.4}"))
                            .unwrap_or_default();
                        println!(
                            "{}",
                            style.dim(&format!(
                                "  {} turns · {}{cost}",
                                outcome.turns,
                                format_usage(&outcome.usage)
                            ))
                        );
                    }
                }

                // Everything else is only interesting in verbose mode, and is
                // already handled by the arms above.
                _ => {}
            }
        }
    })
}

pub fn format_usage(u: &Usage) -> String {
    let mut s = format!("{} in / {} out", u.total_input(), u.output_tokens);
    if u.cache_read_input_tokens > 0 || u.cache_creation_input_tokens > 0 {
        s.push_str(&format!(
            " (cache {} read / {} write)",
            u.cache_read_input_tokens, u.cache_creation_input_tokens
        ));
    }
    s
}

/// The most informative single argument, for the non-verbose tool line.
fn one_line(input: &serde_json::Value) -> String {
    let pick = ["command", "path", "url", "query"]
        .iter()
        .find_map(|k| input.get(*k).and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string(input).unwrap_or_default());
    truncate(&pick.replace('\n', " "), 90)
}

fn first_line(s: &str) -> String {
    truncate(s.lines().next().unwrap_or(""), 200)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

fn indent(s: &str) -> String {
    s.lines().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")
}

fn size_hint(content: &str) -> String {
    let lines = content.lines().count();
    if lines <= 1 {
        format!("{} bytes", content.len())
    } else {
        format!("{lines} lines")
    }
}
