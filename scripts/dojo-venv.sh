#!/usr/bin/env bash
# Build the venv `eval/fixtures/dojo.py` re-executes itself into
# (docs/EXPERIMENT-DESIGN.md §21.2). AgentDojo imports only with its
# LLM-vendor SDKs installed — about 230 MB — so it gets its own interpreter
# rather than the system one. Idempotent; re-run to upgrade.
#
#   scripts/dojo-venv.sh            # ~/.mecha/venvs/dojo
#   MECHA_DOJO_VENV=/elsewhere scripts/dojo-venv.sh
set -euo pipefail
venv="${MECHA_DOJO_VENV:-$HOME/.mecha/venvs/dojo}"
version="${AGENTDOJO_VERSION:-0.1.35}"
if [ ! -x "$venv/bin/python" ]; then
  python3 -m venv "$venv"
fi
"$venv/bin/pip" install --quiet --upgrade pip
"$venv/bin/pip" install --quiet "agentdojo==$version"
"$venv/bin/python" - <<'EOF'
from agentdojo.task_suite.load_suites import get_suite
for name in ("workspace", "banking", "slack", "travel"):
    s = get_suite("v1", name)
    print(f"{name}: {len(s.tools)} tools, {len(s.user_tasks)} user tasks, {len(s.injection_tasks)} injection tasks")
EOF
echo "dojo venv ready at $venv"
