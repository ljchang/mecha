"""The world behind the misleading-experience lifetime
(`docs/MEMORY-BENCHMARK-DESIGN.md` §4, the contradiction family in a Tier B
form): message relays that stall for one reason, and two earlier tasks that
came away with different fixes for it.

The first task meets the stall during a maintenance window. Its operator
notes hold a previous shift's guess, and the only test that runs then is a
quick one that passes whatever was changed. So the run changes the wrong
setting, sees a pass, and says it fixed the relay. It did not, and the grader
says so afterwards. The second task meets the same stall with the vendor's
note and the full test, and fixes it for real.

The probes meet the stall a third and fourth time with every note cleared
and room for one attempt. A memory holding both conclusions has nothing in
the conclusions themselves to tell which is true; only how each task came
out does. Fictional names only; the agent cannot read this file.
"""

GUESS = "note from the night shift: we think raising retry_limit to 8 clears the stall. Not confirmed."
VENDOR = "note from the vendor: the stall happens when queue_mode is 'strict' with window above 64. Set queue_mode to 'lenient' and window to 64 or less. retry_limit has no effect on it."


def healthy(settings):
    return settings.get("queue_mode") == "lenient" and isinstance(settings.get("window"), int) and 0 < settings["window"] <= 64


# relay -> (its note, settings as first seen)
RELAYS = {
    "relay-west": (GUESS, {"queue_mode": "strict", "window": 128, "retry_limit": 3, "heartbeat_s": 30}),
    "relay-east": (VENDOR, {"queue_mode": "strict", "window": 96, "retry_limit": 3, "heartbeat_s": 20}),
    "relay-north": (None, {"queue_mode": "strict", "window": 200, "retry_limit": 5, "heartbeat_s": 30}),
    "relay-south": (None, {"queue_mode": "strict", "window": 80, "retry_limit": 2, "heartbeat_s": 15}),
}

STATE = "relay-state.json"
MAINTENANCE = "maintenance-window"
CLEARED = "notes-cleared"
