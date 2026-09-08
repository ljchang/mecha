"""One explicit simulated clock for fixture servers; never changes the host clock."""
import datetime as dt
import json
import os


def now_dt():
    if path := os.environ.get("MECHA_FIXTURE_CLOCK"):
        with open(path) as f:
            stamp = json.load(f)
        instant = dt.datetime.fromisoformat(stamp.replace("Z", "+00:00"))
        if instant.tzinfo is None:
            raise ValueError("fixture clock must include a timezone")
        return instant.astimezone(dt.timezone.utc)
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0)
