"""Duration parsing for the scheduler."""

import re

_PATTERN = re.compile(r"^(?:(\d+)h)?(?:(\d+)m)?$")


def parse_duration(text):
    """Parse "1h30m", "45m" or "2h" into a whole number of minutes."""
    text = text.strip().lower()
    match = _PATTERN.match(text)
    if not match or not any(match.groups()):
        raise ValueError(f"cannot parse duration {text!r}")

    hours, minutes = match.group(1), match.group(2)
    if minutes is not None:
        return int(minutes)
    return int(hours) * 60
