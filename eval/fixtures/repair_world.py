"""The world behind the split-repair lifetime (`docs/MEMORY-BENCHMARK-DESIGN.md`
§4, Tier B): nightly jobs that fail for a reason only the run log states.

One session can read the log and see the cause. A later session meets the
same fault on another job after every log has been rotated away, and
`jobs_run` says only which step failed. The fix is two settings whose right
values nothing but the log gives, so it cannot be found again by trying.

Two faults, so that a memory holding both still has to tell them apart by
the step that failed. Fictional names only; the agent cannot read this file.
"""

FAULTS = {
    "validate": {
        "step": "step 3 of 5 (validate)",
        "log": "validate: the warehouse stopped accepting schema_version 7 after the spring migration. It accepts schema_version 9, and only with batch_size a multiple of 250.",
        "fixed": lambda s: s.get("schema_version") == 9 and isinstance(s.get("batch_size"), int) and s["batch_size"] > 0 and s["batch_size"] % 250 == 0,
        "said": ("9", "250"),
    },
    "connect": {
        "step": "step 1 of 5 (connect)",
        "log": "connect: region 'eu-2' was retired. Jobs must run in region 'eu-3', and there timeout_s must be at least 900.",
        "fixed": lambda s: s.get("region") == "eu-3" and isinstance(s.get("timeout_s"), (int, float)) and s["timeout_s"] >= 900,
        "said": ("eu-3", "900"),
    },
}

# job -> (fault, settings as first seen)
JOBS = {
    "ingest-harbor": ("validate", {"schema_version": 7, "batch_size": 400, "region": "us-1", "timeout_s": 600, "retention_days": 14}),
    "ingest-juniper": ("connect", {"schema_version": 9, "batch_size": 500, "region": "eu-2", "timeout_s": 300, "retention_days": 30}),
    "ingest-tidewater": ("validate", {"schema_version": 7, "batch_size": 600, "region": "us-1", "timeout_s": 720, "retention_days": 7}),
    "ingest-marlow": ("connect", {"schema_version": 9, "batch_size": 750, "region": "eu-2", "timeout_s": 450, "retention_days": 30}),
}

STATE = "repair-state.json"
ROTATED = "logs-rotated"
