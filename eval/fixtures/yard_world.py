"""The world behind the yard lifetime (`docs/MEMORY-RETENTION-DESIGN.md`
§11.4): a harbour yard whose berth records carry a notice for a while.

An early session reads the records for something routine (lengths, a count
of bollards) and the notices pass under its eyes: nothing about the task
needs them, nothing is refused, nothing fails. A later session has another
job, placing a vessel, and by then the notice board has been cleared. Which
berth is right depends on a notice nobody was asked to keep.

Cold, the placing rule gives a wrong answer every time: "the first berth in
letter order that can take it" is A for a short vessel and C for a long one,
and the notices are what rule those out.

A second board (`second`) reverses one notice, for the lifetime that asks
whether a superseded fact is still handed over.

Fictional names only; the agent cannot read this file.
"""

# berth -> (length in metres, bollards, whether reefer points are fitted)
#
# The points are in the record for good: which berths have them is never in
# doubt, only whether they are live, which a notice says. The first form left
# them out, and a run told that A's and B's plugs were dead placed a reefer
# vessel at C, of which nothing was known either way: an answer the notices
# did not rule out, graded wrong.
BERTHS = {
    "A": (80, 6, True),
    "B": (120, 8, True),
    "C": (200, 14, False),
    "D": (100, 7, True),
    "E": (150, 10, False),
    "F": (110, 8, False),
}

# What each board says, by berth. A berth with no entry has no notice.
BOARDS = {
    "first": {
        "A": "Shore power cabinet burnt out. Reefer plugs are dead, with no date for repair.",
        "B": "Reefer plugs live and tested.",
        "C": "Fender line condemned. No vessel over 90 m alongside until it is relaid.",
        "D": "Crane rail greased. No restriction.",
        "E": "Night lighting on the east apron repaired.",
        "F": "Bollard 4 repainted.",
    },
    "second": {
        "A": "Shore power cabinet burnt out. Reefer plugs are dead, with no date for repair.",
        "B": "Shore power cabinet failed on test. Reefer plugs are dead until further notice.",
        "C": "Fender line condemned. No vessel over 90 m alongside until it is relaid.",
        "D": "New shore power cabinet commissioned. Reefer plugs live and tested.",
        "E": "Night lighting on the east apron repaired.",
        "F": "Bollard 4 repainted.",
    },
    "cleared": {},
}

# vessel -> (length in metres, what it carries)
VESSELS = {
    "Corvid": (70, "refrigerated containers; needs live reefer plugs at the berth"),
    "Halcyon": (140, "timber"),
    "Petrel": (70, "refrigerated containers; needs live reefer plugs at the berth"),
}

STATE = "yard-state.json"
BOARD = "yard-board"
