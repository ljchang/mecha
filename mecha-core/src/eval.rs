//! Grading agent runs.
//!
//! Built for one job: deciding which model to run locally. Final text is a poor
//! signal for that — what matters is whether the model picked the right tool,
//! passed well-formed arguments, and stopped when it should have. So cases are
//! graded on the **tool-call trace** first and the text second.
//!
//! Cases are deliberately read-only. That makes them reproducible, safe to run
//! at high concurrency against a fixture workspace, and repeatable across
//! models — which is the whole point of a bake-off.

use crate::batch::{BatchItem, BatchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub prompt: String,
    #[serde(default)]
    pub expect: Expect,
    /// Free-form labels. The scorecard breaks results down by tag, which is how
    /// you see *where* a model falls over rather than just how often.
    #[serde(default)]
    pub tags: Vec<String>,
}

impl EvalCase {
    pub fn to_item(&self) -> BatchItem {
        BatchItem { id: self.id.clone(), prompt: self.prompt.clone(), meta: None }
    }
}

/// What a correct run looks like. Every populated field becomes one check;
/// a case passes only if all of its checks pass.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Expect {
    /// These tools must each be called at least once, in any order.
    pub tools: Vec<String>,
    /// These tools must be called in this relative order (other calls may be
    /// interleaved). Use for genuine dependencies, not incidental sequence.
    pub tools_in_order: Vec<String>,
    /// These tools must never be called.
    pub forbid_tools: Vec<String>,
    /// No tool may be called at all — the discrimination test. A model that
    /// reaches for a tool to answer "what is 2+2" will waste turns on real work.
    pub no_tools: bool,
    /// Case-insensitive substrings that must appear in the final answer.
    pub contains: Vec<String>,
    /// Case-insensitive substrings that must not appear.
    pub not_contains: Vec<String>,
    /// At least one of these must appear. Use when several phrasings are
    /// equally correct — grading a model down for word choice measures nothing.
    pub contains_any: Vec<String>,
    /// Argument-level assertions.
    pub args: Vec<ArgExpect>,
    /// Fail if the run took more turns than this — catches models that flail.
    pub max_turns: Option<u32>,
}

/// An assertion about the arguments of a particular tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArgExpect {
    pub tool: String,
    /// Argument name, e.g. `path`.
    pub key: String,
    /// The stringified argument must equal this exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<String>,
    /// The stringified argument must contain this (case-insensitive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contains: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradedCase {
    pub id: String,
    pub passed: bool,
    pub tags: Vec<String>,
    pub checks: Vec<Check>,
    pub turns: u32,
    pub elapsed_ms: u64,
    pub malformed_tool_args: u32,
    pub unknown_tools: u32,
    pub tool_errors: u32,
    pub tools_called: Vec<String>,
    pub usage: crate::message::Usage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub text: String,
}

/// Normalize prose before substring matching.
///
/// Models format freely, and raw substring matching measures formatting rather
/// than correctness. Two cases caught this in practice: a model answered
/// `$2,520` and failed a check for `2520`, and answered `do **not** agree` and
/// failed a check for `not agree`. Both answers were right.
///
/// So: fold case, drop markdown emphasis, remove digit-group separators, and
/// collapse whitespace. Applied to needle and haystack alike.
fn normalize(s: &str) -> String {
    let lowered = s.to_lowercase();
    let chars: Vec<char> = lowered.chars().collect();
    let mut out = String::with_capacity(chars.len());

    for (i, &c) in chars.iter().enumerate() {
        match c {
            // Markdown emphasis can land in the middle of a phrase.
            '*' | '_' | '`' | '#' => continue,
            // A separator *between digits* only: "2,520" -> "2520", but
            // "apples, oranges" keeps its comma.
            ',' if i > 0
                && chars[i - 1].is_ascii_digit()
                && chars.get(i + 1).is_some_and(char::is_ascii_digit) =>
            {
                continue
            }
            _ => out.push(c),
        }
    }

    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Grade one result against its case.
pub fn grade(case: &EvalCase, result: &BatchResult) -> GradedCase {
    let mut checks = Vec::new();
    let called: Vec<String> = result.tool_calls.iter().map(|c| c.name.clone()).collect();
    let text_lower = normalize(&result.text);

    // A run that errored outright fails everything; report it once rather than
    // emitting a wall of derived failures.
    if let Some(error) = &result.error {
        checks.push(Check {
            name: "run".into(),
            passed: false,
            detail: error.clone(),
        });
    }

    for tool in &case.expect.tools {
        let passed = called.iter().any(|c| c == tool);
        checks.push(Check {
            name: format!("calls {tool}"),
            passed,
            detail: if passed { String::new() } else { format!("called: {}", fmt(&called)) },
        });
    }

    if !case.expect.tools_in_order.is_empty() {
        let passed = is_subsequence(&case.expect.tools_in_order, &called);
        checks.push(Check {
            name: format!("order {}", case.expect.tools_in_order.join(" → ")),
            passed,
            detail: if passed { String::new() } else { format!("called: {}", fmt(&called)) },
        });
    }

    for tool in &case.expect.forbid_tools {
        let passed = !called.iter().any(|c| c == tool);
        checks.push(Check {
            name: format!("avoids {tool}"),
            passed,
            detail: if passed { String::new() } else { format!("called {tool}") },
        });
    }

    if case.expect.no_tools {
        let passed = called.is_empty();
        checks.push(Check {
            name: "answers without tools".into(),
            passed,
            detail: if passed { String::new() } else { format!("called: {}", fmt(&called)) },
        });
    }

    for needle in &case.expect.contains {
        let passed = text_lower.contains(&normalize(needle));
        checks.push(Check {
            name: format!("says {needle:?}"),
            passed,
            detail: if passed { String::new() } else { "not in the answer".into() },
        });
    }

    for needle in &case.expect.not_contains {
        let passed = !text_lower.contains(&normalize(needle));
        checks.push(Check {
            name: format!("omits {needle:?}"),
            passed,
            detail: if passed { String::new() } else { "present in the answer".into() },
        });
    }

    if !case.expect.contains_any.is_empty() {
        let passed = case
            .expect
            .contains_any
            .iter()
            .any(|n| text_lower.contains(&normalize(n)));
        checks.push(Check {
            name: format!("says one of {}", fmt(&case.expect.contains_any)),
            passed,
            detail: if passed { String::new() } else { "none present".into() },
        });
    }

    for expect in &case.expect.args {
        checks.push(grade_arg(expect, result));
    }

    if let Some(max) = case.expect.max_turns {
        let passed = result.turns <= max;
        checks.push(Check {
            name: format!("≤{max} turns"),
            passed,
            detail: if passed { String::new() } else { format!("took {}", result.turns) },
        });
    }

    // Always graded, whatever the case asks for: a malformed argument or a
    // hallucinated tool name is a failure no matter what the answer said.
    let unknown_tools = result.tool_calls.iter().filter(|c| c.unknown).count() as u32;
    if result.malformed_tool_args > 0 {
        checks.push(Check {
            name: "well-formed arguments".into(),
            passed: false,
            detail: format!("{} call(s) had unparseable JSON", result.malformed_tool_args),
        });
    }
    if unknown_tools > 0 {
        checks.push(Check {
            name: "no invented tools".into(),
            passed: false,
            detail: format!("{unknown_tools} call(s) named a nonexistent tool"),
        });
    }

    GradedCase {
        id: case.id.clone(),
        passed: checks.iter().all(|c| c.passed),
        tags: case.tags.clone(),
        checks,
        turns: result.turns,
        elapsed_ms: result.elapsed_ms,
        malformed_tool_args: result.malformed_tool_args,
        unknown_tools,
        tool_errors: result.tool_calls.iter().filter(|c| c.is_error && !c.unknown).count() as u32,
        tools_called: called,
        usage: result.usage.clone(),
        error: result.error.clone(),
        text: result.text.clone(),
    }
}

fn grade_arg(expect: &ArgExpect, result: &BatchResult) -> Check {
    let name = format!("{}.{}", expect.tool, expect.key);

    let values: Vec<String> = result
        .tool_calls
        .iter()
        .filter(|c| c.name == expect.tool)
        .filter_map(|c| c.input.get(&expect.key).map(stringify))
        .collect();

    if values.is_empty() {
        return Check {
            name,
            passed: false,
            detail: format!("no call to {} passed `{}`", expect.tool, expect.key),
        };
    }

    // Any matching call satisfies the assertion — a model that reads three
    // files including the right one has still found the right one.
    let passed = values.iter().any(|v| {
        expect.equals.as_ref().is_none_or(|e| v == e)
            && expect
                .contains
                .as_ref()
                .is_none_or(|c| normalize(v).contains(&normalize(c)))
    });

    Check {
        name,
        passed,
        detail: if passed { String::new() } else { format!("got {}", fmt(&values)) },
    }
}

/// JSON strings compare as their contents, not with quotes around them.
fn stringify(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn fmt(items: &[String]) -> String {
    if items.is_empty() {
        "(none)".into()
    } else {
        items.join(", ")
    }
}

/// Is `needle` a subsequence of `haystack`?
fn is_subsequence(needle: &[String], haystack: &[String]) -> bool {
    let mut it = haystack.iter();
    needle.iter().all(|want| it.any(|got| got == want))
}

/// Aggregate view of one model's run over the whole case set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scorecard {
    pub model: String,
    pub provider: String,
    pub total: usize,
    pub passed: usize,
    /// Checks passed / checks attempted. Partial credit, unlike `passed`.
    pub check_pass_rate: f64,
    pub malformed_tool_args: u32,
    pub unknown_tools: u32,
    pub tool_errors: u32,
    pub runs_errored: usize,
    pub mean_turns: f64,
    /// Median is the honest latency number here — one 900s timeout would
    /// dominate a mean and tell you nothing about typical behaviour.
    pub median_latency_ms: u64,
    pub total_usage: crate::message::Usage,
    pub wall_clock_ms: u64,
    pub by_tag: Vec<TagScore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagScore {
    pub tag: String,
    pub passed: usize,
    pub total: usize,
}

impl Scorecard {
    pub fn of(
        graded: &[GradedCase],
        model: String,
        provider: String,
        wall_clock_ms: u64,
    ) -> Self {
        let total = graded.len();
        let passed = graded.iter().filter(|g| g.passed).count();

        let checks_total: usize = graded.iter().map(|g| g.checks.len()).sum();
        let checks_passed: usize =
            graded.iter().map(|g| g.checks.iter().filter(|c| c.passed).count()).sum();

        let mut latencies: Vec<u64> = graded.iter().map(|g| g.elapsed_ms).collect();
        latencies.sort_unstable();

        let mut usage = crate::message::Usage::default();
        for g in graded {
            usage.add(&g.usage);
        }

        // Tags in first-seen order, so the scorecard reads in the order the
        // case file declares them rather than alphabetically.
        let mut tags: Vec<String> = Vec::new();
        for g in graded {
            for t in &g.tags {
                if !tags.contains(t) {
                    tags.push(t.clone());
                }
            }
        }
        let by_tag = tags
            .into_iter()
            .map(|tag| {
                let cases: Vec<_> = graded.iter().filter(|g| g.tags.contains(&tag)).collect();
                TagScore {
                    passed: cases.iter().filter(|g| g.passed).count(),
                    total: cases.len(),
                    tag,
                }
            })
            .collect();

        Scorecard {
            model,
            provider,
            total,
            passed,
            check_pass_rate: if checks_total == 0 {
                1.0
            } else {
                checks_passed as f64 / checks_total as f64
            },
            malformed_tool_args: graded.iter().map(|g| g.malformed_tool_args).sum(),
            unknown_tools: graded.iter().map(|g| g.unknown_tools).sum(),
            tool_errors: graded.iter().map(|g| g.tool_errors).sum(),
            runs_errored: graded.iter().filter(|g| g.error.is_some()).count(),
            mean_turns: if total == 0 {
                0.0
            } else {
                graded.iter().map(|g| g.turns as f64).sum::<f64>() / total as f64
            },
            median_latency_ms: latencies.get(latencies.len() / 2).copied().unwrap_or(0),
            total_usage: usage,
            wall_clock_ms,
            by_tag,
        }
    }

    pub fn pass_rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.passed as f64 / self.total as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::ToolCallTrace;
    use serde_json::json;

    fn result_with(calls: Vec<ToolCallTrace>, text: &str) -> BatchResult {
        BatchResult {
            id: "c".into(),
            ok: true,
            text: text.into(),
            error: None,
            turns: 2,
            usage: Default::default(),
            stop_reason: None,
            meta: None,
            elapsed_ms: 10,
            tool_calls: calls,
            malformed_tool_args: 0,
        }
    }

    fn call(name: &str, input: Value) -> ToolCallTrace {
        ToolCallTrace { name: name.into(), input, is_error: false, denied: false, unknown: false }
    }

    fn case(expect: Expect) -> EvalCase {
        EvalCase { id: "c".into(), prompt: "p".into(), expect, tags: vec![] }
    }

    #[test]
    fn formatting_does_not_decide_correctness() {
        // Every one of these is a right answer that raw substring matching
        // marked wrong. All three were observed from a real model.
        let cases = [
            ("Eshin worked 42 hours, for a total of **$2,520**.", "2520"),
            ("Jin's week 28 cost is **$1,750**.", "1750"),
            ("They do **not** agree: README says 1.85.", "not agree"),
            ("The port is `8431`.", "8431"),
        ];
        for (answer, needle) in cases {
            let c = case(Expect { contains: vec![needle.into()], ..Default::default() });
            let r = result_with(vec![], answer);
            assert!(grade(&c, &r).passed, "{answer:?} should satisfy {needle:?}");
        }
    }

    #[test]
    fn normalizing_does_not_make_wrong_answers_pass() {
        let c = case(Expect { contains: vec!["2520".into()], ..Default::default() });
        assert!(!grade(&c, &result_with(vec![], "The total is $2,530.")).passed);

        // A comma between words is not a digit separator and must survive.
        let c = case(Expect { contains: vec!["apples, oranges".into()], ..Default::default() });
        assert!(grade(&c, &result_with(vec![], "We have apples, oranges.")).passed);
        assert!(!grade(&c, &result_with(vec![], "We have apples and oranges.")).passed);
    }

    #[test]
    fn argument_check_matches_any_call_to_that_tool() {
        let c = case(Expect {
            args: vec![ArgExpect {
                tool: "fs_read".into(),
                key: "path".into(),
                equals: Some("README.md".into()),
                contains: None,
            }],
            ..Default::default()
        });
        // The right file is read second; that still counts.
        let r = result_with(
            vec![
                call("fs_read", json!({"path": "Cargo.toml"})),
                call("fs_read", json!({"path": "README.md"})),
            ],
            "",
        );
        assert!(grade(&c, &r).passed);
    }

    #[test]
    fn ordering_allows_interleaved_calls_but_not_reversal() {
        let c = case(Expect {
            tools_in_order: vec!["fs_list".into(), "fs_read".into()],
            ..Default::default()
        });

        let interleaved = result_with(
            vec![
                call("fs_list", json!({})),
                call("http_fetch", json!({})),
                call("fs_read", json!({})),
            ],
            "",
        );
        assert!(grade(&c, &interleaved).passed);

        let reversed =
            result_with(vec![call("fs_read", json!({})), call("fs_list", json!({}))], "");
        assert!(!grade(&c, &reversed).passed);
    }

    #[test]
    fn malformed_arguments_fail_even_when_the_answer_is_right() {
        let c = case(Expect { contains: vec!["hello".into()], ..Default::default() });
        let mut r = result_with(vec![], "hello there");
        r.malformed_tool_args = 1;

        let graded = grade(&c, &r);
        assert!(!graded.passed);
        assert!(graded.checks.iter().any(|ch| ch.name == "well-formed arguments"));
    }

    /// The shipped case set must stay loadable — a typo in one line would
    /// otherwise only surface partway through a paid eval run.
    #[test]
    fn shipped_cases_all_parse() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("eval/cases.jsonl");
        let text = std::fs::read_to_string(&path).expect("eval/cases.jsonl is missing");

        let mut ids = std::collections::HashSet::new();
        let mut count = 0;
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            let case: EvalCase = serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("cases.jsonl:{}: {e}", i + 1));
            assert!(!case.prompt.trim().is_empty(), "case {} has no prompt", case.id);
            assert!(!case.tags.is_empty(), "case {} has no tags", case.id);
            assert!(ids.insert(case.id.clone()), "duplicate case id {}", case.id);
            count += 1;
        }
        assert!(count >= 15, "expected a substantive case set, found {count}");
    }

    #[test]
    fn no_tools_catches_a_model_that_reaches_for_one() {
        let c = case(Expect { no_tools: true, ..Default::default() });
        assert!(grade(&c, &result_with(vec![], "4")).passed);
        assert!(!grade(&c, &result_with(vec![call("shell", json!({}))], "4")).passed);
    }
}
