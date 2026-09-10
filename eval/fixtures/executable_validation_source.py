#!/usr/bin/env python3
"""Corrective-task source using the existing independent artifact oracle."""
import json
from pathlib import Path
import appraisal_mismatch_source as source
source.CASES = json.loads((Path(__file__).resolve().parent / "executable-validation/cases.json").read_text())
# No required call order, tool name, or model self-report establishes success.
source.tasks = lambda: [dict(id=c["id"], prompt=c["prompt"], tags=["executable-validation"])
                        for c in source.CASES]
if __name__ == "__main__": source.main()
