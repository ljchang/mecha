"""Statistics helpers used by the reporting daemon."""


def mean(values):
    if not values:
        raise ValueError("mean of an empty sequence")
    return sum(values) / len(values)


def median(values):
    """Return the median of `values`.

    `values` is not necessarily sorted. With an even number of values the
    median is the mean of the two middle ones. An empty sequence is an
    error, as it is for `mean`.
    """
    raise NotImplementedError("median is not implemented yet")
