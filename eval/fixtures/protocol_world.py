"""The two worlds behind the far-analog lifetime
(`docs/MEMORY-BENCHMARK-DESIGN.md` §4, "far analog"): one hidden protocol,
two services that share no tool, no parameter name and no wording.

The protocol: a thing must be taken before it can be changed, and given back
afterwards. A change without taking it is refused; a thing left taken is a
failure nobody reports at the time.

- **registry** (experience). Records, with documentation that states both
  halves, and tool names that mean what they do.
- **quay** (probe). Berths, with no documentation and five operations named
  by letter, one of which takes a berth and one of which gives it back. The
  refusal says the berth is not held, and `quay_find` shows whether it is,
  so the taking can be found by trying. Nothing says a berth must be given
  back: only the earlier service did.

A memory indexed by tool, or searched by the task's words, has nothing to
offer the second service. What carries is the arrangement: one call enables
another on the same thing, and a third must follow. Fictional names only; the
agent cannot read this file.
"""

REGISTRY = {
    "records": {
        "Marlow ledger": ("R-7", {"owner": "Voss", "tier": 2}),
        "Juniper deed": ("R-12", {"owner": "Arden", "tier": 1}),
    },
    "topics": {
        "": "Topics: update, records. Ask for one by name.",
        "records": "A record is addressed by its id. reg_find turns a name into an id and shows the record.",
        # Both halves in the one topic a refused update sends a reader to. In
        # the first baseline they were two topics, the model read one, and
        # half the experience tasks never met the second half: nothing to
        # carry, which is a different measurement.
        "update": "reg_update changes one field of a record. It is refused with E4 unless the record is locked with reg_lock first. Always reg_unlock when you are done: a record left locked blocks everyone else and fails the nightly audit.",
    },
}

QUAY = {
    "berths": {
        "North pier three": ("B-3", {"draft": 6, "crane": "east"}),
        "South pier one": ("B-1", {"draft": 4, "crane": "west"}),
    },
    # The operation that takes a berth and the one that gives it back. The
    # other three do nothing.
    "hold": "quay_op_d",
    "free": "quay_op_b",
    "ops": ["quay_op_a", "quay_op_b", "quay_op_c", "quay_op_d", "quay_op_e"],
}

STATE = "protocol-state.json"
WORLD = "world"
