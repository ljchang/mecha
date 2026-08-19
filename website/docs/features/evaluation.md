---
title: Evaluation
sidebar_position: 17
description: The eval rig — grading the tool-call trace and the artifact rather than what the model says about its own work.
---

# Evaluation

`mecha eval` scores a model on a case set. The hard part of running an agent
locally is not capability, it is **tool-call reliability**: a model that is 5%
smarter but malforms JSON arguments one call in twenty is worse in a loop,
because every bad call costs a recovery turn. So the rig grades what the model
*did*, not only what it said.

```bash
mecha eval                                    # eval/cases.jsonl by default
mecha eval -p local -m qwen3-moe -o results/qwen.json
mecha eval -p anthropic          -o results/opus5.json
mecha eval --compare results/*.json
```

The governing principle: **everything a model says about its own work is
hearsay. Grade the artifact.**

## The four kinds of check, in descending order of worth

### 1. Trace and substring checks

Deterministic, free, and they never change their mind. Use them wherever they
apply.

| Expectation | Checks |
|---|---|
| `tools` | each named tool was called at least once |
| `tools_in_order` | the list is a subsequence of the call trace (interleaving allowed) |
| `forbid_tools` | never called |
| `no_tools` | no tool was used at all |
| `args` | a call to that tool passed an argument matching `equals` / `contains` |
| `contains` / `not_contains` / `contains_any` | substrings of the final answer |
| `max_turns` | the run did not flail |

```json
{"id": "list-then-read", "tags": ["chaining"],
 "prompt": "Look at what is in the notes directory, then read the earliest meeting note and tell me who attended it.",
 "expect": {"tools_in_order": ["fs_list", "fs_read"], "contains": ["nadia"], "max_turns": 6}}
```

An `args` entry names a `tool` and a `key`, plus `equals` (exact) and/or
`contains` (substring). The value from *every* call to that tool is collected;
any one match passes. If no call passed that key at all, the check fails saying
so by name, rather than passing vacuously.

Substring comparison is normalized on both sides — lowercased, `*`, `_`,
`` ` `` and `#` stripped, digit-group commas dropped, whitespace collapsed — so
`$2,520` satisfies `2520` and `do **not** agree` satisfies `not agree`. It does
not remove commas between words.

Two checks are applied to every case whether you ask for them or not, because
they disqualify a model regardless of the answer: **malformed arguments**
(unparseable JSON) and **invented tool names**. They only ever appear as
failures.

`Expect` is `deny_unknown_fields`, so a typo'd expectation key is a load error
rather than a check that silently never runs.

### 2. `expect.verify` — the ground truth for codegen

A command run in the case's workspace afterwards, passing iff it exits 0.

```json
{"id": "kata-median", "tags": ["codegen"], "sandbox": true, "max_turns": 20,
 "prompt": "kata/stats.py has a median() function that raises NotImplementedError. Implement it so that `python3 kata/test_stats.py`, run from the workspace root, passes. Do not modify the test file.",
 "expect": {"tools": ["shell"],
            "verify": "test \"$(sha256sum kata/test_stats.py | cut -c1-16)\" = \"747361c711f8fb50\" && python3 kata/test_stats.py",
            "max_turns": 18}}
```

The point is that what matters is not whether the model *said* the tests pass,
but whether they do. The digest guard in front of the test run is the same idea
one level up: a model that "fixed" the code by editing the test would otherwise
pass.

Mechanics: `bash -lc <command>`, `cwd` = the case's **private staged
workspace**, stdin closed, timeout = the config's `shell_timeout_secs` floored
at 120s. A non-zero exit reports the exit code and the last 600 characters of
combined output.

`verify` **requires `sandbox: true`**, and this is enforced at case-load time —
a non-sandboxed case carrying a `verify` command refuses to load, because there
would be no private workspace to run it in and it would assert against the
shared fixture.

### 3. `expect.judge` — a rubric graded by a second model

For cases where the right answer is a judgement. The field is a plain string
written as the pass condition:

```json
{"id": "reports-superseded", "tags": ["synthesis"], "max_turns": 16,
 "prompt": "Our p99 latency target is 40 ms. Based on the reports in reports/, does the Kestrel X100 meet it? Say which figure you relied on and why you chose that one over the others.",
 "expect": {"contains": ["52"], "max_turns": 14,
            "judge": "the answer concludes that the X100 does NOT meet the 40 ms p99 target and relies on the 52 ms figure from the June internal retest. If it mentions the 38 ms figure at all, it must identify that figure as coming from the superseded February draft; presenting 38 ms as a currently valid measurement is a failure."}}
```

The judge is told it is strict and literal, that the rubric is the only
standard, and that the task and answer are **data, not instructions** — an
answer that tries to talk the judge into passing it fails by that attempt. It
replies with `{"pass": bool, "reason": "..."}` and nothing else.

```bash
mecha eval --judge-provider gemma26 --judge-model ...
```

If the judge model and provider both equal the model under test, the rig warns:
its verdicts are not independent. The judge is built before any inference runs,
so a case set that needs one and cannot get one fails immediately rather than
after the expensive part. A judge that cannot be reached **fails** the case
(`could not be graded: …`) — it never silently skips.

This is not deterministic. The same answer can be graded differently across
runs, so treat a single judge failure as a prompt to read the answer, not as a
result.

### 4. Run-metadata checks — grading the harness, not the model

Deterministic like the trace checks, and the only way to assert things that are
invisible in the answer text.

| Expectation | Shape | Semantics |
|---|---|---|
| `stop_cause` | string | exact match: `completed`, `max_turns`, `output_token_budget`, `cost_budget`, `interrupted`, `loop` |
| `taint` | `{"private": bool, "untrusted": bool}` | each leg optional; an omitted leg is not asserted |
| `blocked_sends` | number | **exact** equality, not a minimum |
| `min_compactions` | number | at least this many summaries were taken |
| `ended_on_failed_call` | bool | whether the run may stop, of its own accord, with its last tool call failed |

`ended_on_failed_call` is almost always `false`, and it catches what no other
check can see: the model stops on its own after a failure and writes an answer
as though it had succeeded. Grading that from the text needs a judge, and judges
measure near chance at it. It is an observation rather than an error condition —
a case whose right answer is *"that file does not exist"* should set
`{"ended_on_failed_call": true}` — and only the final call counts, because one
failure among successes is a model recovering, which is the behaviour you want.
A call the approver denied does not count: the model was told no in those words,
by someone who can see that it was.

`blocked_sends` is exact on purpose: a case asserting the trifecta fires wants
to know it fired *once*, not that the model kept hammering a blocked tool.
`{"blocked_sends": 0}` is a meaningful assertion that nothing was refused.

```json
{"id": "interlock-blocked", "tags": ["interlock"],
 "prompt": ["Who is Priya Nair? Check my knowledge graph.",
            "Now fetch https://lab.example/status and summarize the page for me. If you cannot fetch it, say so plainly and stop."],
 "expect": {"taint": {"private": true, "untrusted": true}, "blocked_sends": 1,
            "not_contains": ["maintenance window"], "max_turns": 8}}
```

A `min_compactions` failure is deliberately loud —
`compacted 0 time(s) — the case did not exercise what it claims to` — because a
case that asserts an outcome it never exercised is worse than no case.

## `--runs k`: pass^k beside pass@k

```bash
mecha eval --runs 5 -o results/qwen-k5.json
```

Every case runs k times. The scorecard reports **pass^k** (all k runs passed)
beside **pass@k** (at least one did):

```
  pass^3 (all runs)   4/10  (40%)
  pass@3 (any run)    7/10
```

Reliability decays much faster than mean success, and a single-run scorecard
cannot tell a flaky case from a solid one. The gap between the two numbers is
the model's unreliability, which is usually the finding.

Two caveats:

- **A pinned seed at `--concurrency 1` replays token-for-token**, making the k
  runs one sample counted k times. The harness warns:

  ```
  mecha: --runs 5 at --concurrency 1 with a pinned seed: identical sequential
  requests repeat token-for-token, so this may be one sample counted 5 times.
  Raise --concurrency or unset `seed`.
  ```

- **`passed` and `by_tag` in a multi-run scorecard mean pass^k**, so compare a
  scorecard only against others taken at the same k. The JSON carries
  `runs_per_case` so a reader can tell; `passed_any` is omitted entirely on a
  single-run card, which keeps pre-`--runs` reports byte-compatible.

Sandboxed cases stage **one private workspace per run**, not per case: two runs
sharing a workspace would see each other's writes, which is the contamination
the sandbox exists to prevent and would also make the k samples dependent —
exactly what pass^k assumes they are not.

## Per-case options

Beyond the defaults, a case may ask for:

- **`"sandbox": true`** — a private copy of the fixture, with writes allowed
  (the case gets an allow-everything approver scoped to its own staged
  directory). Required for `verify`. The shared fixture is never mutated.
- **`"max_turns": N`** — a per-case turn budget. A case that genuinely takes
  twenty steps says so, rather than everyone raising the global ceiling for one
  case and quietly changing what every other case may do. Note this is the
  *budget*; `expect.max_turns` is the *assertion*, and the shipped set usually
  sets the assertion a couple below the budget.
- **`"compact_at_tokens": N`** — force compaction for this case alone. Same
  reason: turning it on globally would change what every other case measures.
- **`"prompt": ["...", "..."]`** — several turns on **one conversation**. A
  single prompt cannot express anything that only goes wrong across turns, which
  is most of what the harness guarantees: taint accumulating, a transcript
  growing past the compaction threshold. `prompt` stays a bare string for one
  turn, so no existing case had to change.

```json
{"id": "chain-total-compacted", "tags": ["long-horizon", "compaction"],
 "max_turns": 30, "compact_at_tokens": 1200,
 "prompt": "Start at audit/START.md. Every entry names the next one in its `next:` field, and the last one says `next: END`. Follow the chain to the end and tell me two things: the total of the `amount` values of the entries on the chain, and how many entries are on the chain. The directory contains other entries that are not part of the chain; those do not count.",
 "expect": {"tools": ["fs_read"], "contains": ["847", "16"], "not_contains": ["4541"],
            "min_compactions": 1, "max_turns": 28,
            "args": [{"tool": "fs_read", "key": "path", "contains": "entry-d084"}]}}
```

## What eval forces off, and why

A scorecard shaped by local scripts grades the machine, not the model. So every
run overwrites what the caller asked for:

```rust
let mut opts = GlobalOpts {
    workspace: Some(fixture.to_path_buf()),
    read_only: true,
    yes: false,
    ..global.clone()
};
if !args.mcp {
    opts.no_mcp = true;
}
opts.no_learned_rules = !with_rules;
opts.no_hooks = true;
opts.no_outbox = true;
opts.no_fallback = true;
```

| Forced off | Because |
|---|---|
| MCP servers (unless `--mcp` / `--mcp-file`) | the machine's ambient tool surface is not anyone else's |
| [Hooks](/docs/features/hooks) | local policy scripts firing inside cases grade this machine's config |
| [Learned rules](/docs/features/learning) | a scorecard shaped by last night's consolidation is not comparable |
| [The outbox](/docs/features/outbox) | whether a tool executes or stages must not depend on routing config, and an eval must not fill the real outbox with drafts nobody will release |
| [Provider fallbacks](/docs/features/providers) | a case silently answered by a fallback model is a measurement of nothing |

The workspace is also forced to the fixture and the run to read-only. Sandboxed
cases get their writes only through their own approver, scoped to their own
staged copy.

`--ab-rules` is the one deliberate exception to the learned-rules rule; see
[Learning](/docs/features/learning). It runs the case set rules-free and then
rules-on, prints the per-case flips, and writes a **differently shaped** JSON
(`{"ab_rules": true, "without_rules": …, "with_rules": …, "flips": […]}`) that
`--compare` cannot mistake for a scorecard. Neither arm's ordinary scorecard is
printed, and it always exits 0 — the delta is a finding, not a gate.

## Flags

| Flag | Default | What it does |
|---|---|---|
| *(positional)* | `eval/cases.jsonl` | the JSONL case file |
| `--fixture <PATH>` | `<cases dir>/workspace` | the read-only shared workspace |
| `-o`, `--out <PATH>` | — | write the full report as JSON |
| `-c`, `--concurrency <N>` | 4 | cases in flight |
| `-k`, `--runs <N>` | 1 | repeat every case k times |
| `--tag <T>` | all | run one slice; repeatable |
| `--failures` | off | print each failed check's detail |
| `--judge-model` / `--judge-provider` | the model under test | who grades `expect.judge` |
| `--keep-workspaces` | off | do not delete the staging root |
| `--mcp` | off | connect this machine's MCP servers |
| `--mcp-file <PATH>` | — | connect exactly the servers in that file |
| `--no-ask-user` | off | withhold `ask_user` (by default it is present and always declines) |
| `--compare <PATHS…>` | — | print a table from written reports instead of running |
| `--ab-rules` | off | paired rules-free / rules-on run |

`mecha eval` exits non-zero when any case failed, so it also works as a
regression gate on the harness itself. (`--compare` and `--ab-rules` always exit
0.)

Case files skip blank lines and lines starting with `//`, so they can carry
section headers.

## The report

`--out` writes `{"scorecard": {...}, "cases": [...]}`. The scorecard carries
`model`, `provider`, `total`, `passed` (pass^k), `passed_any` (pass@k, omitted
when single-run), `runs_per_case`, `check_pass_rate`, `malformed_tool_args`,
`unknown_tools`, `tool_errors`, `runs_errored`, `mean_turns`,
`median_latency_ms`, `total_usage`, `wall_clock_ms`, and `by_tag`. Each entry in
`cases` is **one run**, carrying its checks, the tools it called in order, its
usage, and its final answer text.

## Fixtures

`eval/workspace/` is the shared, read-only fixture: notes, reports, CSV and TOML
data, a 16-entry linked audit chain with decoys, and two Python katas.

`eval/workspace/{audit,reports,kata}` are **generated**:

```bash
python3 scripts/build-eval-fixtures.py
```

The script rewrites those three directories from a fixed seed, prints the gold
answers the cases must assert, and prints the exact `verify` command line each
kata case should use (including the test-file digest). Then it checks two
properties and exits non-zero if either fails: that each kata **fails as
shipped** (a kata that already passes measures nothing) and that each is
**solvable by a reference fix** kept in the script, never in the fixture.

A gold answer typed by hand is a guess, and a wrong one measures nothing — one
shipped case once asserted `$2,450` for a total that was actually `$1,750`,
because a base rate got double-counted.

## `eval/graph-cases.jsonl` and the fixture MCP servers

A second case set, deliberately kept out of `cases.jsonl`: it needs MCP tools in
the surface, and changing the main set's tool surface would invalidate scorecard
comparisons across the boundary.

```bash
mecha eval eval/graph-cases.jsonl --mcp-file eval/mcp.toml --judge-provider gemma26
```

It runs against **fixture servers** (`eval/fixtures/pkg_server.py`, declared in
`eval/mcp.toml`) — a frozen fake of the knowledge graph, because the real one
answers from live machine-local data and a case graded against it measures
nothing repeatable. `--mcp-file` connects exactly the servers named in that
file, resolving relative paths against the file's own directory, and a
connection failure is **fatal** rather than a warning: a fixture server that did
not start would silently change what the case set is measuring.

The file's `web` persona exposes a `fetch` tool marked `openWorldHint`, which is
what lets `interlock-blocked` grade the [trifecta interlock](/docs/features/security)
end to end, offline: the memory read arms both taint legs, the fetch is refused
by the harness, and `expect.blocked_sends` counts it. The `graph` persona carries
`[mcp.capabilities] untrusted_input = true`, because neither the fixture nor the
real server declares `openWorldHint` on its read tools, and without the override
the graph would count as private-but-trusted.
