"""Run me with: python3 kata/test_stats.py"""

from stats import mean, median


def check(actual, expected, label):
    assert actual == expected, f"{label}: expected {expected!r}, got {actual!r}"


check(median([3, 1, 2]), 2, "odd length, unsorted")
check(median([1, 2, 3, 4]), 2.5, "even length")
check(median([7]), 7, "single value")
check(median([5, 5, 5, 5]), 5, "all equal")
check(median([-3, -1, -2]), -2, "negatives")
check(mean([1, 2, 3]), 2, "mean still works")

try:
    median([])
except ValueError:
    pass
else:
    raise AssertionError("median([]) should raise ValueError")

print("ok")
