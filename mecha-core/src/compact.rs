//! Making a long conversation fit.
//!
//! Every turn sends the whole history, so a session that runs long enough stops
//! being able to send anything at all. Compaction replaces the middle of the
//! transcript with a summary and keeps the ends: the task at the top, so the
//! agent still knows what it was asked, and the most recent turns verbatim,
//! because that is where the work actually is.
//!
//! ## The constraint that decides the design
//!
//! A `tool_result` is only valid if its `tool_use` is still in the conversation
//! — the next request 400s otherwise, and that is the whole run gone. So the
//! cut cannot land anywhere convenient; it has to land somewhere *legal*. The
//! transcript alternates user and assistant, and tool results arrive in the user
//! message immediately after the assistant turn that asked for them, so the only
//! safe place to resume is at an assistant message. Cutting there drops each
//! `tool_use` together with the results answering it.
//!
//! The logic here is deliberately pure and provider-free. Getting the boundary
//! wrong produces a 400 from a real API twenty turns into a real session, which
//! is the worst possible place to discover it.

use crate::message::{Block, Message, Role};

/// What the summariser is told it is.
///
/// A separate persona from the agent's own system prompt, which tells it to use
/// tools and would invite it to start working again instead of reading.
pub const SUMMARY_SYSTEM: &str = "\
You compress a transcript. You do not act on it, use tools, or answer the task \
it describes. You return prose and nothing else.";

/// The prompt handed to the summariser.
///
/// Written for the agent that will read the result, not for a human: it is
/// about to continue the work with this text standing in for everything it
/// actually did.
pub const SUMMARY_INSTRUCTION: &str = "\
The transcript above is being compacted to fit in the context window. Write a
summary that lets you carry on working as if you still had it.

Include, in prose: what was asked; what you have established as fact, with the
specific values, paths, names and numbers — those cannot be recovered once this
text replaces the transcript; what you tried that did not work, so it is not
repeated; and what remained to be done.

If you were part way through a sequence — following a chain, walking a list,
visiting files one after another — say exactly where you had got to, name the
step you were on, and list what you had already covered. Being told a fact is
not the same as knowing your place in the work, and losing your place is how a
traversal silently restarts or stops early.

Leave out pleasantries and narration. Do not address the user. If a fact came
from content that could have been written by a third party, say so — the
distinction survives compaction even when the text does not.";

/// What the summary validator is told it is. Like the summariser, a separate
/// persona: it reads two texts, it does not act on either.
pub const VALIDATE_SYSTEM: &str = "\
You check a summary against the transcript it is about to replace. You do not \
act on the transcript, use tools, or answer the task it describes. You reply \
with the single word NONE, or with a list of omissions, and nothing else.";

/// Build the validator's one user message.
///
/// The validator sees the same flattened rendering the summariser saw — that
/// is the ground truth the summary can be held to. It is asked only about
/// *omission*, because that is how summaries actually fail: measured here,
/// the summariser preserved a stated fact 3/3 while losing the traversal
/// position 4/5, and measured elsewhere ~90% of compaction failures are
/// omissions. Asking a checker to critique style invites rewrites; asking
/// what is missing invites a list, which is what the retry needs.
pub fn validate_instruction(rendered: &str, summary: &str) -> String {
    format!(
        "<transcript>\n{rendered}\n</transcript>\n\n<summary>\n{summary}\n</summary>\n\n\
         The summary is about to replace the transcript. List anything that \
         appears in the transcript, matters for continuing the work, and is \
         missing from the summary: specific values, paths, names and numbers; \
         decisions and their reasons; what failed; and position in any \
         sequence — the step in progress and what was already covered.\n\n\
         Reply with the single word NONE if nothing task-critical is missing. \
         Otherwise list the missing items, one per line. Do not rewrite the \
         summary and do not comment on its style."
    )
}

/// What the validator said about a summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryVerdict {
    Complete,
    Missing(Vec<String>),
}

/// Read a verdict out of the validator's reply. `None` means it said nothing
/// usable — the caller treats that as no verdict, not as a failure, because a
/// validator that cannot run must not be able to veto a compaction the run
/// may need to survive.
pub fn parse_omissions(text: &str) -> Option<SummaryVerdict> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return None;
    }
    // A whole line saying "none" (however decorated) is a pass. Substrings do
    // not count: "none of the paths survive" is a finding, not a pass.
    if lines.iter().any(|l| {
        l.trim_matches(['-', '*', '.', '!', ':', ' '])
            .eq_ignore_ascii_case("none")
    }) {
        return Some(SummaryVerdict::Complete);
    }
    Some(SummaryVerdict::Missing(
        lines
            .iter()
            .map(|l| l.trim_start_matches(['-', '*', ' ']).to_string())
            .collect(),
    ))
}

/// The summariser's second attempt: the same instruction, plus what the first
/// attempt lost. Naming the omissions is the whole intervention — the
/// summariser cannot see its own gaps, and a bare "try again" would sample
/// the same blind spot.
pub fn retry_instruction(omissions: &[String]) -> String {
    format!(
        "{SUMMARY_INSTRUCTION}\n\nA check of your previous summary against the \
         transcript found it omitted the following. The rewritten summary must \
         include them:\n{}",
        omissions
            .iter()
            .map(|o| format!("- {o}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// Flatten messages into plain text for the summariser.
///
/// Deliberately *not* a replay of the structured transcript. Sending the real
/// messages means sending `tool_result`s with no tools declared on the request,
/// and llama-server answers that with an empty completion — found by running it,
/// not by reading the spec. Prose has no such failure mode on any provider, and
/// it also removes any chance of the summariser deciding to call something.
pub fn render_for_summary(messages: &[Message], max_result_chars: usize) -> String {
    let mut out = String::new();

    for message in messages {
        let who = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        for block in &message.content {
            match block {
                Block::Text { text } if !text.trim().is_empty() => {
                    out.push_str(&format!("[{who}] {}\n", text.trim()));
                }
                Block::ToolUse { name, input, .. } => {
                    out.push_str(&format!("[assistant calls {name}] {input}\n"));
                }
                Block::ToolResult {
                    content, is_error, ..
                } => {
                    let label = if *is_error {
                        "tool error"
                    } else {
                        "tool result"
                    };
                    out.push_str(&format!("[{label}] {}\n", clip(content, max_result_chars)));
                }
                // Named, never carried. The summariser is a *tool-less
                // prose* call, so the base64 could only arrive as a
                // megabyte of literal text in a request whose whole purpose
                // is to be smaller than what it replaces — and the model
                // reading it has no way to know it is looking at an image.
                // What survives a compaction is that one was here and what
                // it was called, which is exactly what `recall` then needs
                // to find the turn again.
                Block::Image {
                    media_type, source, ..
                } => {
                    out.push_str(&format!(
                        "[{who}] {}\n",
                        Block::image_placeholder(media_type, source.as_deref())
                    ));
                }
                // Reasoning is the model talking to itself and does not survive
                // into the next turn anyway.
                Block::Thinking { .. } | Block::Text { .. } => {}
            }
        }
    }
    out
}

fn clip(s: &str, max: usize) -> String {
    let flat = s.trim();
    if flat.chars().count() <= max {
        return flat.to_string();
    }
    format!(
        "{}… [{} characters omitted]",
        flat.chars().take(max).collect::<String>(),
        flat.chars().count() - max
    )
}

/// The first index at or after `target` where the transcript can be cut.
///
/// Returns `None` when there is no legal cut, which is normal for a short
/// conversation and means "do not compact" rather than "something is wrong".
pub fn cut_point(messages: &[Message], target: usize) -> Option<usize> {
    // Index 0 is the original task and is kept regardless, so a cut there would
    // drop nothing and gain nothing.
    (target.max(1)..messages.len()).find(|&i| is_safe_cut(messages, i))
}

/// Can the conversation resume at `i` without orphaning anything?
///
/// Only at an assistant message. A user message may carry `tool_result` blocks
/// answering the assistant turn before it; resuming there would leave those
/// results referring to a `tool_use` that no longer exists.
fn is_safe_cut(messages: &[Message], i: usize) -> bool {
    messages.get(i).is_some_and(|m| m.role == Role::Assistant)
}

/// Marks the block holding tool state carried across a compaction.
///
/// A sentinel rather than a convention: [`rebuild`] finds the previous carried
/// block by this prefix and *replaces* it. Without that, a second compaction
/// would leave last hour's task list sitting in the prompt above this one's,
/// and a model reading two contradictory lists is worse off than one reading
/// neither.
pub const CARRIED_HEADER: &str =
    "[Live state, carried past the compaction and current as of now — it supersedes \
     anything about it in the summaries above:]";

/// Rebuild the transcript around `summary`.
///
/// The original task keeps its place at the top with the summary appended to
/// it, rather than the summary becoming a message of its own — two user
/// messages in a row are rejected by some providers, and the task and the
/// summary of what happened to it belong together anyway.
///
/// `carried` is `(label, body)` state a tool asked to keep verbatim (see
/// `Tool::carried_state`). It goes *after* the summary, because it is the one
/// part of the rebuilt head that is known to be current rather than
/// paraphrased, and last is where a model reads most carefully.
pub fn rebuild(
    messages: &[Message],
    cut: usize,
    summary: &str,
    carried: &[(&str, &str)],
) -> Vec<Message> {
    let mut out = Vec::with_capacity(messages.len() - cut + 1);

    let mut head = messages[0].clone();
    // Drop the carried block a previous compaction left. Summaries accumulate
    // on purpose — each describes a different stretch of the conversation —
    // but there is only ever one *current* state, and keeping the old copy
    // would be keeping a wrong one.
    head.content.retain(|block| match block {
        Block::Text { text } => !text.trim_start().starts_with(CARRIED_HEADER),
        _ => true,
    });
    head.content.push(Block::text(format!(
        "\n\n[Earlier turns were compacted to fit the context window. What \
         happened in them:]\n{summary}"
    )));
    if !carried.is_empty() {
        let mut block = format!("\n\n{CARRIED_HEADER}\n");
        for (label, body) in carried {
            block.push_str(&format!("\n## {label}\n{}\n", body.trim_end()));
        }
        head.content.push(Block::text(block));
    }
    out.push(head);

    out.extend(messages[cut..].iter().cloned());
    out
}

/// Appended to a result whose middle was removed, so the model can tell the
/// difference between a short file and a shortened one.
pub const TRUNCATION_MARKER: &str = "\n… [earlier output truncated to save context]";

/// How much of a tool result survives thinning.
///
/// Generous enough that a small file — the common case in agent work — is kept
/// whole, and the head is where structured output puts the part worth having.
pub const THINNED_RESULT_CHARS: usize = 240;

/// Shorten old tool *results*, leaving the tool *calls* that produced them.
///
/// This is the cheap half of compaction and it should be tried first, because a
/// call and its result differ enormously in both size and value:
///
/// ```text
/// tool_use    {"path": "entry-9e1b.md"}          ~15 tokens  ← the position
/// tool_result "# Audit entry 11\namount: 43…"     ~80 tokens  ← the bulk
/// ```
///
/// Position lives in the calls, which are tiny. Tokens live in the results,
/// which are not. Replacing the middle of a transcript wholesale throws away
/// both, which is why a summarised traversal loses its place: the agent can no
/// longer see which entries it already visited. Thinning keeps that sequence
/// *structurally*, so it does not depend on a summariser noticing it mattered.
///
/// Costs no request, so it can run before deciding whether a summary is needed
/// at all. Returns how many results were shortened.
pub fn thin_old_results(messages: &mut [Message], keep_recent: usize, keep_chars: usize) -> usize {
    let cutoff = messages.len().saturating_sub(keep_recent);
    let mut thinned = 0;

    for message in messages.iter_mut().take(cutoff) {
        for block in &mut message.content {
            let Block::ToolResult { content, .. } = block else {
                continue;
            };
            // Already thinned: leave it, or repeated passes would eat the head
            // a chunk at a time.
            if content.ends_with(TRUNCATION_MARKER) || content.chars().count() <= keep_chars {
                continue;
            }
            let head: String = content.chars().take(keep_chars).collect();
            *content = format!("{head}{TRUNCATION_MARKER}");
            thinned += 1;
        }
    }
    thinned
}

/// Starts every evicted result, so a second pass can tell it has already been
/// here — and so the model can tell a stale result from a short one.
pub const SUPERSEDED_MARKER: &str = "[stale:";

/// Replace tool results that a later call has superseded.
///
/// Runs before thinning, and before any summary, because it is the only pass
/// here that *removes damage* rather than trading tokens for fidelity: a
/// superseded read is semantically related to the current state of the work
/// and wrong about it, which is measurably worse than irrelevant bulk —
/// related-but-wrong distractors cost 25–68% where unrelated content is
/// near-free. A transcript holding two versions of the same file is exactly
/// that shape, and deleting the old one is lossless: the newest result still
/// says everything the transcript knows to be true.
///
/// What counts as "the same target":
///
/// - A string `path` argument, across tools — so an `fs_write` supersedes an
///   earlier `fs_read` of the file it just changed, which is the case the
///   distractor research names directly.
/// - Otherwise the tool name plus its exact arguments — the model asked the
///   same question twice, and the newer answer speaks for both.
///
/// Errors neither supersede nor get evicted: a failed call does not describe
/// the target's state, and "what failed" is what keeps it from being retried.
/// Returns how many results were evicted.
pub fn evict_superseded_results(messages: &mut [Message]) -> usize {
    // Which result answered each call, and whether it errored.
    let mut errored = std::collections::HashMap::new();
    for message in messages.iter() {
        for block in &message.content {
            if let Block::ToolResult {
                tool_use_id,
                is_error,
                ..
            } = block
            {
                errored.insert(tool_use_id.clone(), *is_error);
            }
        }
    }

    // Every call in transcript order; the last non-error call per target is
    // the authoritative one.
    let mut calls: Vec<(String, String, String)> = Vec::new(); // (id, tool, target)
    for message in messages.iter() {
        for block in &message.content {
            if let Block::ToolUse { id, name, input } = block {
                calls.push((id.clone(), name.clone(), target_of(name, input)));
            }
        }
    }
    let mut authoritative: std::collections::HashMap<&str, &str> = Default::default();
    for (id, _, target) in &calls {
        if errored.get(id) == Some(&false) {
            authoritative.insert(target, id);
        }
    }
    // The tool behind the authoritative call, for the marker text.
    let superseder: std::collections::HashMap<&str, &str> = calls
        .iter()
        .filter(|(id, _, target)| authoritative.get(target.as_str()) == Some(&id.as_str()))
        .map(|(_, name, target)| (target.as_str(), name.as_str()))
        .collect();

    let call_of: std::collections::HashMap<&str, (&str, &str)> = calls
        .iter()
        .map(|(id, name, target)| (id.as_str(), (name.as_str(), target.as_str())))
        .collect();

    let mut evicted = 0;
    for message in messages.iter_mut() {
        for block in &mut message.content {
            let Block::ToolResult {
                tool_use_id,
                content,
                is_error,
            } = block
            else {
                continue;
            };
            if *is_error || content.starts_with(SUPERSEDED_MARKER) {
                continue;
            }
            let Some(&(name, target)) = call_of.get(tool_use_id.as_str()) else {
                continue;
            };
            // Superseded means a *different, later* call owns the target now.
            match authoritative.get(target) {
                Some(&winner) if winner != tool_use_id => {
                    let later = superseder.get(target).copied().unwrap_or(name);
                    // Name the recovery: a marker that only says "gone" leaves
                    // the model to conclude the content never existed.
                    *content = format!(
                        "{SUPERSEDED_MARKER} a later {later} call covered the same \
                         target, so this older result no longer reflects it. The \
                         newest result is authoritative; call {name} again if this \
                         content is needed.]"
                    );
                    evicted += 1;
                }
                _ => {}
            }
        }
    }
    evicted
}

/// Starts every collapsed repeat, so a second pass can tell it has already
/// been here — and so the model reads a marker instead of its own failure a
/// fourth time.
pub const REPEAT_MARKER: &str = "[repeat:";

/// The refusals this pass must never touch.
///
/// A denied call carries `is_error: true` like any failure, so keying the
/// collapse on that flag alone would fold a *human's* refusals together — and
/// these exact prefixes are what `learning.rs` and `counterfactual.rs` strip
/// to mine a correction. Three "no"s to the same command would then reach the
/// miner as one, and the transcript that recorded them is rewritten in place,
/// so the evidence is gone rather than merely uncounted.
///
/// Matched on the result text because that is all a `tool_result` carries —
/// the `denied` flag lives on the trace, which compaction never sees. The
/// strings are the loop's own (`agent.rs`), which is what makes this a
/// duplication worth a test on both sides rather than a shared constant: the
/// loop chooses the label from the `Decision` variant, and this pass must
/// follow whatever it chose.
const REFUSAL_PREFIXES: &[&str] = &[
    "Denied by the user:",
    "Blocked by policy:",
    "Blocked by a hook:",
];

/// Is this result a person or a policy saying no, rather than the environment
/// failing?
fn is_refusal(content: &str) -> bool {
    REFUSAL_PREFIXES.iter().any(|p| content.starts_with(p))
}

/// Collapse a pile of identical failures down to its newest member.
///
/// Errors are exempt from [`evict_superseded_results`] on purpose: a failed
/// call says nothing about the target, and *what failed* is what stops it
/// being retried. That rule is right for one failure and inverts for eight.
/// A model is measurably more likely to fail a step when the context holds
/// its own earlier errors — self-conditioning, which does not go away with
/// model size ("Measuring Long Horizon Execution in LLMs", ICLR 2026) — and a
/// repeated failure is the same-target near-miss that `CONTEXT-RESEARCH.md`
/// §1 puts at 25–68% harm, not the free kind of bulk. The diagnosis the
/// exemption exists to protect is carried by the **newest** failure on its
/// own; the copies behind it are a corpus the model wrote about its own
/// incompetence.
///
/// So the newest failure per target survives verbatim and the older identical
/// ones become markers. Three decisions:
///
/// - **The key is the target *and* the exact error text**, on the loop
///   guard's precedent (identical call *and* identical result). Two different
///   failures on one path — "no such file", then "permission denied" — are two
///   facts, and folding either into a count loses one. Collapsing too little
///   costs a few tokens; collapsing too much destroys a diagnosis, so the
///   narrow key is the fail-safe direction.
/// - **Nothing is removed.** A `tool_result` whose `tool_use` is gone is a
///   400, so dropping the block is not available at any price; the content is
///   replaced in place, exactly as eviction does it. What this pass removes is
///   the *repetition*, which is the mechanism — not the bytes.
/// - **It is not the loop guard.** That one stops a run which has already gone
///   wrong, and only after a compaction. This runs before there is anything to
///   stop.
///
/// Returns how many results were collapsed.
pub fn collapse_repeated_failures(messages: &mut [Message]) -> usize {
    // What each call was about, so a result can be keyed by its target rather
    // than by the id that is unique to the attempt.
    let mut target_of_call: std::collections::HashMap<String, String> = Default::default();
    for message in messages.iter() {
        for block in &message.content {
            if let Block::ToolUse { id, name, input } = block {
                target_of_call.insert(id.clone(), target_of(name, input));
            }
        }
    }

    // The newest failure per (target, message). Transcript order, so the last
    // write wins — and the last write is the one kept whole.
    let mut newest: std::collections::HashMap<(String, String), String> = Default::default();
    let key_of = |tool_use_id: &String, content: &String, is_error: bool| {
        if !is_error || content.starts_with(REPEAT_MARKER) || is_refusal(content) {
            return None;
        }
        let target = target_of_call.get(tool_use_id)?;
        Some((target.clone(), content.trim().to_string()))
    };
    for message in messages.iter() {
        for block in &message.content {
            if let Block::ToolResult {
                tool_use_id,
                content,
                is_error,
            } = block
            {
                if let Some(key) = key_of(tool_use_id, content, *is_error) {
                    newest.insert(key, tool_use_id.clone());
                }
            }
        }
    }

    let mut collapsed = 0;
    for message in messages.iter_mut() {
        for block in &mut message.content {
            let Block::ToolResult {
                tool_use_id,
                content,
                is_error,
            } = block
            else {
                continue;
            };
            let Some(key) = key_of(tool_use_id, content, *is_error) else {
                continue;
            };
            match newest.get(&key) {
                Some(latest) if latest != tool_use_id => {
                    // Name what happened and what it means: a marker that only
                    // says "collapsed" invites the model to try once more to
                    // see for itself.
                    *content = format!(
                        "{REPEAT_MARKER} this call failed again later with the same error, \
                         which is kept in full below. Repeating it unchanged has not worked.]"
                    );
                    collapsed += 1;
                }
                _ => {}
            }
        }
    }
    collapsed
}

/// What a call is *about*, for supersession.
fn target_of(name: &str, input: &serde_json::Value) -> String {
    match input.get("path").and_then(serde_json::Value::as_str) {
        // Deliberately not prefixed with the tool name: the newest operation
        // on a path speaks for the path, whichever tool performed it. But a
        // *ranged* read speaks only for its slice — `offset`/`limit` join the
        // key, or reading lines 100–110 would evict the full read of the same
        // file, and successive range reads (exactly what the spillover marker
        // tells the model to do) would evict each other while holding
        // different content. A write carries no range, so it still supersedes
        // the unranged read.
        Some(path) => format!(
            "path\u{0}{path}\u{0}{}\u{0}{}",
            input
                .get("offset")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            input
                .get("limit")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        ),
        // `serde_json::Map` is a BTreeMap, so this string is canonical even if
        // the model orders the arguments differently between calls.
        None => format!("{name}\u{0}{input}"),
    }
}

/// Whether compacting would actually remove anything worth the round trip.
///
/// A summarising call costs a request and its tokens; doing it to drop two
/// messages loses on both counts.
pub fn worth_compacting(messages: &[Message], cut: usize) -> bool {
    cut > MIN_DROPPED && messages.len() > cut
}

/// Below this, the summary is likely to be longer than what it replaces.
const MIN_DROPPED: usize = 4;

/// Every `tool_use` id in the transcript that has no matching `tool_result`.
///
/// The invariant compaction must never break, exposed so it can be asserted on
/// rather than assumed.
pub fn orphaned_tool_uses(messages: &[Message]) -> Vec<String> {
    let mut answered = Vec::new();
    let mut asked = Vec::new();

    for message in messages {
        for block in &message.content {
            match block {
                Block::ToolUse { id, .. } => asked.push(id.clone()),
                Block::ToolResult { tool_use_id, .. } => answered.push(tool_use_id.clone()),
                _ => {}
            }
        }
    }
    asked
        .into_iter()
        .filter(|id| !answered.contains(id))
        .collect()
}

/// Every `tool_result` whose `tool_use` is missing — the error that 400s.
pub fn orphaned_tool_results(messages: &[Message]) -> Vec<String> {
    let mut asked = Vec::new();
    let mut orphans = Vec::new();

    for message in messages {
        for block in &message.content {
            match block {
                Block::ToolUse { id, .. } => asked.push(id.clone()),
                Block::ToolResult { tool_use_id, .. } if !asked.contains(tool_use_id) => {
                    orphans.push(tool_use_id.clone())
                }
                _ => {}
            }
        }
    }
    orphans
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str, path: &str) -> Message {
        Message::assistant(vec![Block::ToolUse {
            id: id.into(),
            name: "fs_read".into(),
            input: serde_json::json!({"path": path}),
        }])
    }

    fn result(id: &str, body: &str) -> Message {
        Message::tool_results(vec![Block::ToolResult {
            tool_use_id: id.into(),
            content: body.into(),
            is_error: false,
        }])
    }

    /// A traversal: read a file, get its contents, move on.
    fn walk(n: usize) -> Vec<Message> {
        let mut m = vec![Message::user("follow the chain")];
        for i in 0..n {
            m.push(call(&format!("t{i}"), &format!("entry-{i}.md")));
            m.push(result(&format!("t{i}"), &"x".repeat(500)));
        }
        m
    }

    #[test]
    fn thinning_keeps_every_call_and_shortens_only_the_results() {
        let mut m = walk(8);
        let before_calls: Vec<_> = m
            .iter()
            .flat_map(|m| m.tool_uses())
            .map(|(_, _, i)| i.clone())
            .collect();

        let thinned = thin_old_results(&mut m, 4, 240);

        assert!(thinned > 0);
        // The sequence of calls is what says where the agent got to, and it is
        // untouched — that is the whole point of thinning rather than cutting.
        let after_calls: Vec<_> = m
            .iter()
            .flat_map(|m| m.tool_uses())
            .map(|(_, _, i)| i.clone())
            .collect();
        assert_eq!(
            before_calls, after_calls,
            "thinning disturbed the tool calls"
        );
        assert_eq!(m.len(), 17, "thinning removed messages");
    }

    #[test]
    fn recent_results_are_left_alone() {
        let mut m = walk(8);
        thin_old_results(&mut m, 4, 240);

        let last_result = m.last().unwrap().content.iter().find_map(|b| match b {
            Block::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        });
        assert_eq!(
            last_result.unwrap().len(),
            500,
            "the newest result was thinned"
        );
    }

    #[test]
    fn thinning_is_idempotent() {
        // Repeated passes must not eat the surviving head a chunk at a time —
        // compaction runs every turn once the threshold is crossed.
        let mut m = walk(8);
        thin_old_results(&mut m, 4, 240);
        let after_one: Vec<String> = m.iter().map(|m| format!("{:?}", m.content)).collect();

        let second = thin_old_results(&mut m, 4, 240);
        let after_two: Vec<String> = m.iter().map(|m| format!("{:?}", m.content)).collect();

        assert_eq!(second, 0, "a second pass thinned already-thinned results");
        assert_eq!(after_one, after_two);
    }

    fn body_of(message: &Message) -> String {
        message
            .content
            .iter()
            .find_map(|b| match b {
                Block::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn a_verdict_parses_through_the_ways_models_actually_phrase_it() {
        use SummaryVerdict::*;
        // Passes, however decorated.
        for text in ["NONE", "none", "None.", "**NONE**", "Verdict:\nNONE"] {
            assert_eq!(parse_omissions(text), Some(Complete), "{text:?}");
        }
        // "none" as a substring is a finding, not a pass.
        let found = parse_omissions("none of the file paths survive the summary").unwrap();
        assert!(matches!(found, Missing(_)));

        // Omission lists come back, bullets stripped, ready for the retry.
        let found = parse_omissions("- the amount 847\n- the path audit/entry-d084.md").unwrap();
        assert_eq!(
            found,
            Missing(vec![
                "the amount 847".into(),
                "the path audit/entry-d084.md".into()
            ])
        );

        // Nothing usable is no verdict — the caller must not treat it as a
        // veto, because a run may need this compaction to survive.
        assert_eq!(parse_omissions(""), None);
        assert_eq!(parse_omissions("   \n  "), None);
    }

    #[test]
    fn the_retry_instruction_names_every_omission_and_keeps_the_original_brief() {
        let retry = retry_instruction(&["the amount 847".into(), "the QX-4417 reference".into()]);
        assert!(
            retry.contains(SUMMARY_INSTRUCTION),
            "the retry must still say how to summarise"
        );
        assert!(retry.contains("- the amount 847"));
        assert!(retry.contains("- the QX-4417 reference"));
    }

    #[test]
    fn a_rereads_earlier_copy_is_evicted_and_the_newest_survives_whole() {
        // Read the same file twice: the first copy is the near-miss distractor
        // — same path, same symbols, possibly wrong content — and the second
        // says everything the transcript knows to be true.
        let mut m = vec![
            Message::user("go"),
            call("t0", "a.md"),
            result("t0", "old contents"),
            call("t1", "a.md"),
            result("t1", "new contents"),
        ];
        assert_eq!(evict_superseded_results(&mut m), 1);
        assert!(body_of(&m[2]).starts_with(SUPERSEDED_MARKER));
        assert!(
            body_of(&m[2]).contains("fs_read"),
            "the marker names the recovery"
        );
        assert_eq!(
            body_of(&m[4]),
            "new contents",
            "the authoritative copy was touched"
        );
    }

    #[test]
    fn a_write_supersedes_an_earlier_read_of_the_same_path() {
        // The exact shape the distractor research names: a file read left in
        // context after an edit changed the file. The read is now wrong.
        let mut m = vec![
            Message::user("go"),
            call("t0", "a.md"),
            result("t0", "pre-edit contents"),
            Message::assistant(vec![Block::ToolUse {
                id: "t1".into(),
                name: "fs_write".into(),
                input: serde_json::json!({"path": "a.md", "content": "post"}),
            }]),
            result("t1", "wrote 4 bytes"),
        ];
        assert_eq!(evict_superseded_results(&mut m), 1);
        assert!(body_of(&m[2]).starts_with(SUPERSEDED_MARKER));
        assert!(
            body_of(&m[2]).contains("fs_write"),
            "the marker says what superseded it"
        );
    }

    #[test]
    fn errors_neither_supersede_nor_get_evicted() {
        let mut m = vec![
            Message::user("go"),
            call("t0", "a.md"),
            result("t0", "good contents"),
            call("t1", "a.md"),
            Message::tool_results(vec![Block::ToolResult {
                tool_use_id: "t1".into(),
                content: "permission denied".into(),
                is_error: true,
            }]),
        ];
        // The later *failed* read says nothing about the file; the good copy
        // must survive, and the failure must stay so it is not retried.
        assert_eq!(evict_superseded_results(&mut m), 0);
        assert_eq!(body_of(&m[2]), "good contents");
        assert_eq!(body_of(&m[4]), "permission denied");
    }

    #[test]
    fn a_pile_of_identical_failures_collapses_to_its_newest_member() {
        // The self-conditioning shape: the model retries the same call four
        // times, fails identically every time, and every copy stays in the
        // context conditioning the next attempt. Before this pass, all four
        // survived verbatim — eviction skips errors and thinning only
        // truncates long results outside the recent window, and a failure
        // message is short.
        let mut m = vec![Message::user("go")];
        for i in 0..4 {
            m.push(call(&format!("t{i}"), "a.md"));
            m.push(err_result(&format!("t{i}"), "permission denied"));
        }

        assert_eq!(collapse_repeated_failures(&mut m), 3);
        for i in 0..3 {
            assert!(
                body_of(&m[2 + i * 2]).starts_with(REPEAT_MARKER),
                "attempt {i} was left to condition the next one"
            );
        }
        assert_eq!(
            body_of(&m[8]),
            "permission denied",
            "the newest failure must survive whole — it is the diagnosis that \
             stops the call being retried"
        );
    }

    #[test]
    fn a_persons_repeated_refusals_are_never_collapsed() {
        // The learning miner strips "Denied by the user:" to build a
        // correction, and compaction rewrites the transcript in place — so
        // folding three denials into one marker does not merely undercount
        // them, it destroys the evidence. A denied call carries `is_error`
        // like any failure, which is exactly why this needs its own rule.
        let mut m = vec![Message::user("go")];
        for i in 0..3 {
            m.push(call(&format!("t{i}"), "secrets.env"));
            m.push(err_result(
                &format!("t{i}"),
                "Denied by the user: not that file",
            ));
        }
        assert_eq!(collapse_repeated_failures(&mut m), 0);
        for i in 0..3 {
            assert_eq!(
                body_of(&m[2 + i * 2]),
                "Denied by the user: not that file",
                "a refusal the miner reads was overwritten"
            );
        }

        // The machine's own refusals are equally untouched: they are not
        // environment failures either, and one of them being mistaken for a
        // user correction is the mistake this project has a test for already.
        for prefix in ["Blocked by policy:", "Blocked by a hook:"] {
            let mut m = vec![Message::user("go")];
            for i in 0..3 {
                m.push(call(&format!("t{i}"), "a.md"));
                m.push(err_result(&format!("t{i}"), &format!("{prefix} no")));
            }
            assert_eq!(collapse_repeated_failures(&mut m), 0, "{prefix}");
        }

        // And the pass still does its job beside them: an environment failure
        // repeated three times in the same transcript still collapses.
        let mut m = vec![Message::user("go")];
        for i in 0..3 {
            m.push(call(&format!("d{i}"), "denied.md"));
            m.push(err_result(&format!("d{i}"), "Denied by the user: no"));
            m.push(call(&format!("e{i}"), "gone.md"));
            m.push(err_result(&format!("e{i}"), "no such file"));
        }
        assert_eq!(collapse_repeated_failures(&mut m), 2);
    }

    #[test]
    fn two_different_failures_on_one_target_are_two_facts() {
        // "no such file" and "permission denied" say different things about
        // a.md. Folding either into a count loses a diagnosis, which is the
        // damage the error exemption exists to prevent — so the key is the
        // error text as well as the target.
        let mut m = vec![
            Message::user("go"),
            call("t0", "a.md"),
            err_result("t0", "no such file"),
            call("t1", "a.md"),
            err_result("t1", "permission denied"),
        ];
        assert_eq!(collapse_repeated_failures(&mut m), 0);
        assert_eq!(body_of(&m[2]), "no such file");
        assert_eq!(body_of(&m[4]), "permission denied");
    }

    #[test]
    fn identical_failures_on_different_targets_are_left_alone() {
        // Same message, different files: two facts about two paths, not a
        // model repeating itself.
        let mut m = vec![
            Message::user("go"),
            call("t0", "a.md"),
            err_result("t0", "no such file"),
            call("t1", "b.md"),
            err_result("t1", "no such file"),
        ];
        assert_eq!(collapse_repeated_failures(&mut m), 0);
    }

    #[test]
    fn a_successful_result_is_never_collapsed_by_the_failure_pass() {
        // Supersession is eviction's job and it has its own rules; this pass
        // must not quietly become a second, blunter copy of it.
        let mut m = vec![
            Message::user("go"),
            call("t0", "a.md"),
            result("t0", "contents"),
            call("t1", "a.md"),
            result("t1", "contents"),
        ];
        assert_eq!(collapse_repeated_failures(&mut m), 0);
        assert_eq!(body_of(&m[2]), "contents");
    }

    #[test]
    fn collapsing_is_idempotent_and_keeps_every_result_block() {
        // Runs on every compaction, so a second pass must not walk back over
        // its own markers. And a `tool_result` whose `tool_use` is gone is a
        // 400: the count of blocks is not allowed to change, ever.
        let mut m = vec![Message::user("go")];
        for i in 0..3 {
            m.push(call(&format!("t{i}"), "a.md"));
            m.push(err_result(&format!("t{i}"), "permission denied"));
        }
        let blocks = m.len();

        assert_eq!(collapse_repeated_failures(&mut m), 2);
        let after_one: Vec<String> = m.iter().map(|m| format!("{:?}", m.content)).collect();

        assert_eq!(
            collapse_repeated_failures(&mut m),
            0,
            "a second pass collapsed its own markers"
        );
        let after_two: Vec<String> = m.iter().map(|m| format!("{:?}", m.content)).collect();

        assert_eq!(after_one, after_two);
        assert_eq!(m.len(), blocks, "a result block was dropped");
        assert!(orphaned_tool_results(&m).is_empty());
        assert!(orphaned_tool_uses(&m).is_empty());
    }

    fn err_result(id: &str, content: &str) -> Message {
        Message::tool_results(vec![Block::ToolResult {
            tool_use_id: id.into(),
            content: content.into(),
            is_error: true,
        }])
    }

    #[test]
    fn a_ranged_read_speaks_only_for_its_slice() {
        let ranged = |id: &str, offset: u64| {
            Message::assistant(vec![Block::ToolUse {
                id: id.into(),
                name: "fs_read".into(),
                input: serde_json::json!({"path": "big.txt", "offset": offset, "limit": 10}),
            }])
        };
        let mut m = vec![
            Message::user("go"),
            call("t0", "big.txt"), // the full read
            result("t0", "the whole file"),
            ranged("t1", 100),
            result("t1", "lines 100-110"),
            ranged("t2", 200),
            result("t2", "lines 200-210"),
        ];
        // Three different slices of one file: nothing supersedes anything —
        // each result holds content none of the others has.
        assert_eq!(evict_superseded_results(&mut m), 0);

        // The same slice twice is a re-read, and the newest speaks for it.
        m.push(ranged("t3", 100));
        m.push(result("t3", "lines 100-110 again"));
        assert_eq!(evict_superseded_results(&mut m), 1);
        assert!(
            body_of(&m[4]).starts_with(SUPERSEDED_MARKER),
            "the older 100-slice"
        );
        assert_eq!(body_of(&m[2]), "the whole file", "the full read survived");
    }

    #[test]
    fn different_targets_do_not_supersede_each_other() {
        let mut m = vec![
            Message::user("go"),
            call("t0", "a.md"),
            result("t0", "a contents"),
            call("t1", "b.md"),
            result("t1", "b contents"),
        ];
        assert_eq!(evict_superseded_results(&mut m), 0);
    }

    #[test]
    fn identical_non_path_calls_dedup_and_different_arguments_do_not() {
        let shell = |id: &str, cmd: &str| {
            Message::assistant(vec![Block::ToolUse {
                id: id.into(),
                name: "shell".into(),
                input: serde_json::json!({"command": cmd}),
            }])
        };
        let mut m = vec![
            Message::user("go"),
            shell("t0", "cargo test"),
            result("t0", "1 failed"),
            shell("t1", "cargo build"),
            result("t1", "ok"),
            shell("t2", "cargo test"),
            result("t2", "all passed"),
        ];
        // The first `cargo test` is stale — the suite has been re-run since —
        // but `cargo build` asked a different question and keeps its answer.
        assert_eq!(evict_superseded_results(&mut m), 1);
        assert!(body_of(&m[2]).starts_with(SUPERSEDED_MARKER));
        assert_eq!(body_of(&m[4]), "ok");
        assert_eq!(body_of(&m[6]), "all passed");
    }

    #[test]
    fn eviction_is_idempotent_and_never_touches_the_calls() {
        let mut m = vec![
            Message::user("go"),
            call("t0", "a.md"),
            result("t0", "old"),
            call("t1", "a.md"),
            result("t1", "new"),
        ];
        let calls_before: Vec<_> = m
            .iter()
            .flat_map(|m| m.tool_uses())
            .map(|(_, _, i)| i.clone())
            .collect();
        assert_eq!(evict_superseded_results(&mut m), 1);
        assert_eq!(
            evict_superseded_results(&mut m),
            0,
            "a second pass re-evicted"
        );

        let calls_after: Vec<_> = m
            .iter()
            .flat_map(|m| m.tool_uses())
            .map(|(_, _, i)| i.clone())
            .collect();
        assert_eq!(
            calls_before, calls_after,
            "eviction disturbed the tool calls"
        );
        assert!(orphaned_tool_results(&m).is_empty());
        assert!(orphaned_tool_uses(&m).is_empty());
    }

    #[test]
    fn a_result_shorter_than_the_budget_is_not_touched() {
        let mut m = vec![
            Message::user("go"),
            call("t0", "a.md"),
            result("t0", "amount: 43"),
        ];
        assert_eq!(thin_old_results(&mut m, 0, 240), 0);
        assert!(!format!("{:?}", m[2].content).contains("truncated"));
    }

    #[test]
    fn thinning_says_it_thinned_so_the_model_can_tell() {
        // A silently shortened file reads as a short file, and the model would
        // conclude the rest of it does not exist.
        let mut m = walk(2);
        thin_old_results(&mut m, 0, 240);
        let body = m[2].content.iter().find_map(|b| match b {
            Block::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        });
        assert!(body.unwrap().ends_with(TRUNCATION_MARKER));
    }
    use serde_json::json;

    /// A transcript in the shape the loop actually produces: a task, then
    /// alternating assistant tool calls and their results, then an answer.
    fn transcript(turns: usize) -> Vec<Message> {
        let mut messages = vec![Message::user("do the thing")];
        for i in 0..turns {
            messages.push(Message::assistant(vec![Block::ToolUse {
                id: format!("t{i}"),
                name: "echo".into(),
                input: json!({"n": i}),
            }]));
            messages.push(Message::tool_results(vec![Block::ToolResult {
                tool_use_id: format!("t{i}"),
                content: format!("result {i}"),
                is_error: false,
            }]));
        }
        messages.push(Message::assistant(vec![Block::text("done")]));
        messages
    }

    #[test]
    fn a_cut_never_orphans_a_tool_result() {
        // The failure this exists to prevent is a 400 from a real API twenty
        // turns into a real session, so check every target, not a lucky one.
        let messages = transcript(6);
        for target in 0..messages.len() {
            let Some(cut) = cut_point(&messages, target) else {
                continue;
            };
            let rebuilt = rebuild(&messages, cut, "a summary", &[]);

            assert!(
                orphaned_tool_results(&rebuilt).is_empty(),
                "cutting at {cut} (target {target}) orphaned a tool result"
            );
            assert!(
                orphaned_tool_uses(&rebuilt).is_empty(),
                "cutting at {cut} (target {target}) left a tool call unanswered"
            );
        }
    }

    #[test]
    fn the_cut_lands_on_an_assistant_turn_and_at_or_after_the_target() {
        let messages = transcript(5);
        for target in 0..messages.len() {
            let Some(cut) = cut_point(&messages, target) else {
                continue;
            };
            assert!(
                cut >= target.max(1),
                "a cut before the target drops too much"
            );
            assert_eq!(messages[cut].role, Role::Assistant);
        }
    }

    #[test]
    fn the_original_task_survives_and_the_recent_turns_are_verbatim() {
        let messages = transcript(6);
        let cut = cut_point(&messages, 6).unwrap();
        let rebuilt = rebuild(&messages, cut, "we established that X is 42", &[]);

        // The task is still there, so the agent still knows what it is doing.
        assert!(rebuilt[0].text().contains("do the thing"));
        assert!(rebuilt[0].text().contains("X is 42"));
        assert_eq!(rebuilt[0].role, Role::User);

        // ...and the tail was not paraphrased.
        assert_eq!(rebuilt.len(), 1 + messages.len() - cut);
        assert_eq!(
            rebuilt.last().unwrap().text(),
            messages.last().unwrap().text()
        );
    }

    #[test]
    fn the_rebuilt_transcript_never_has_two_user_messages_in_a_row() {
        // Some providers reject it outright, and it is exactly what a naive
        // "prepend the summary as a message" would produce.
        let messages = transcript(6);
        let cut = cut_point(&messages, 5).unwrap();
        let rebuilt = rebuild(&messages, cut, "s", &[]);

        for pair in rebuilt.windows(2) {
            assert!(
                !(pair[0].role == Role::User && pair[1].role == Role::User),
                "consecutive user messages"
            );
        }
    }

    /// The measured failure of summarising is that it keeps what is true and
    /// drops how far you got. A task list is nothing but how far you got, and
    /// it lives in a tool rather than in the messages — so it crosses verbatim.
    #[test]
    fn tool_state_crosses_a_compaction_verbatim() {
        let messages = transcript(6);
        let cut = cut_point(&messages, 6).unwrap();
        let list = "1/3 done\n[x] read the config\n[~] fix the port\n[ ] run the tests\n";
        let rebuilt = rebuild(
            &messages,
            cut,
            "we established that X is 42",
            &[("todo", list)],
        );

        let head = rebuilt[0].text();
        assert!(head.contains("X is 42"), "the summary is still there");
        assert!(head.contains("[~] fix the port"), "{head}");
        assert!(head.contains("[ ] run the tests"), "{head}");
        // After the summary, not before: it is the one part known to be current
        // rather than paraphrased.
        assert!(
            head.find(CARRIED_HEADER).unwrap() > head.find("X is 42").unwrap(),
            "{head}"
        );
    }

    /// The bug a second compaction would otherwise introduce: two task lists in
    /// the prompt, one of them wrong, with nothing to say which.
    #[test]
    fn a_second_compaction_replaces_the_carried_state_rather_than_stacking_it() {
        let messages = transcript(6);
        let cut = cut_point(&messages, 6).unwrap();
        let first = rebuild(&messages, cut, "summary one", &[("todo", "[ ] step one")]);

        // Now compact the already-compacted transcript, as a long run does.
        let cut = cut_point(&first, first.len().saturating_sub(2)).unwrap();
        let second = rebuild(
            &first,
            cut,
            "summary two",
            &[("todo", "[x] step one\n[ ] step two")],
        );

        let head = second[0].text();
        assert_eq!(head.matches(CARRIED_HEADER).count(), 1, "{head}");
        assert!(head.contains("[ ] step two"), "{head}");
        assert!(
            !head.contains("[ ] step one"),
            "last compaction's list survived beside this one's: {head}"
        );
        // Summaries *do* accumulate — each describes a different stretch — and
        // that is the difference being tested.
        assert!(head.contains("summary one") && head.contains("summary two"));
    }

    /// Nothing to carry must produce nothing, not an empty section: a heading
    /// with no list under it reads as "the plan is finished".
    #[test]
    fn no_tool_state_leaves_no_trace() {
        let messages = transcript(6);
        let cut = cut_point(&messages, 6).unwrap();
        let rebuilt = rebuild(&messages, cut, "a summary", &[]);
        assert!(!rebuilt[0].text().contains(CARRIED_HEADER));
    }

    #[test]
    fn a_short_conversation_is_left_alone() {
        let messages = vec![
            Message::user("hi"),
            Message::assistant(vec![Block::text("hello")]),
        ];
        // There is a legal cut, but nothing worth dropping.
        let cut = cut_point(&messages, 1).unwrap();
        assert!(!worth_compacting(&messages, cut));
    }

    #[test]
    fn a_transcript_ending_mid_tool_call_still_cuts_safely() {
        // The shape left behind by an interrupted run: the assistant asked for
        // a tool and the results are the last thing in the transcript.
        let mut messages = transcript(4);
        messages.pop();
        assert_eq!(messages.last().unwrap().role, Role::User);

        let cut = cut_point(&messages, 3).unwrap();
        let rebuilt = rebuild(&messages, cut, "s", &[]);
        assert!(orphaned_tool_results(&rebuilt).is_empty());
    }
}
