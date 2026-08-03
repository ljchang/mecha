"""Run me with: python3 kata/test_parse.py"""

from parse import parse_duration


def check(actual, expected, label):
    assert actual == expected, f"{label}: expected {expected!r}, got {actual!r}"


check(parse_duration("45m"), 45, "minutes only")
check(parse_duration("2h"), 120, "hours only")
check(parse_duration("1h30m"), 90, "hours and minutes")
check(parse_duration(" 3H15M "), 195, "whitespace and case")

for bad in ["", "soon", "90"]:
    try:
        parse_duration(bad)
    except ValueError:
        pass
    else:
        raise AssertionError(f"parse_duration({bad!r}) should raise ValueError")

print("ok")
