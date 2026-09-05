#!/usr/bin/env python3
"""The task-source contract, as small as it can be written (docs/EXPERIMENT-DESIGN.md
§21.1). A manifest names a source instead of a case file:

    [tasks]
    source = ["python3", "eval/fixtures/source_stub.py"]
    fixture = "eval/workspace"

and `mecha exp` calls it with three verbs, on the run child's base environment
plus four pointers — `MECHA_HOME` (the trial home), `MECHA_FIXTURES` (its
fixture stores' root), `MECHA_EXPERIMENT_WORKSPACE` (the trial's jail) and
`MECHA_EXPERIMENT_TASK` (the task id, on `setup` and `grade`):

    list                 -> JSON list of {id, prompt, tags?, max_turns?, expect?}
    setup <task>         -> put the world in the task's starting state; exit 0
    grade <task>         -> stdin: the run's result JSON (`mecha run --json`);
                            stdout: {passed, detail?, checks?: [{name, passed, detail}]}

Everything is fail-closed on the driver's side: a non-zero exit, a timeout,
no JSON, or an unknown shape fails the trial rather than passing it, and a
verdict that disagrees with its own checks is refused. This stub is also
what the driver's tests run against: two tasks, a `setup` that leaves a
marker under the fixtures root, and a `grade` that passes when the answer
says the word the task asked for. `eval/fixtures/dojo.py` is the real one.
"""

import json
import os
import sys

TASKS = [
    {"id": "say-hello", "prompt": "Say exactly: hello", "tags": ["stub"], "word": "hello"},
    {"id": "say-goodbye", "prompt": "Say exactly: goodbye", "tags": ["stub", "farewell"], "word": "goodbye", "max_turns": 2},
]


def find(task_id):
    for t in TASKS:
        if t["id"] == task_id:
            return t
    print(f"source_stub: no task `{task_id}`", file=sys.stderr)
    sys.exit(2)


def main(argv):
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    verb, rest = argv[0], argv[1:]
    if verb == "list":
        out = [{k: v for k, v in t.items() if k != "word"} for t in TASKS]
        json.dump(out, sys.stdout)
        return 0
    if verb == "setup":
        t = find(rest[0])
        root = os.environ.get("MECHA_FIXTURES")
        if root:
            os.makedirs(root, exist_ok=True)
            with open(os.path.join(root, "stub-setup.json"), "w") as f:
                json.dump({"task": t["id"], "workspace": os.environ.get("MECHA_EXPERIMENT_WORKSPACE")}, f)
        return 0
    if verb == "grade":
        t = find(rest[0])
        result = json.load(sys.stdin)
        text = result.get("text", "")
        said = t["word"] in text.lower()
        json.dump(
            {
                "passed": said,
                "detail": f"the answer {'says' if said else 'does not say'} {t['word']!r}",
                "checks": [{"name": f"says {t['word']}", "passed": said, "detail": text[:80]}],
            },
            sys.stdout,
        )
        return 0
    print(f"source_stub: unknown verb `{verb}`", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
