"""The world behind the depot lifetime (`docs/MEMORY-RETENTION-DESIGN.md`
§11.4 and §11.6): a parcel depot whose bay records carry a handover note for
one shift.

The yard lifetime turned out to test memory against a present record: the
later task could read the same berth again, found it silent, and had to
choose. Here the later task cannot look at all. A routine tally reads the
bay records while a note rides in each; a later task needs one thing a note
said (a locker's code, the door a courier uses) or one thing the owner
mentioned in passing (which crate a recipient's parcels go in), and by then
the notes are gone and no tool returns any of the three. The service takes
the code, the door or the crate as an argument and never gives one back.

Cold, a probe cannot be worked out: the code is four digits, the doors are
nine, the crates are eight.

Held out: written on 2026-11-16 with the arms' predictions in the manifest,
before any arm ran on it. Not for tuning.

Fictional names only; the agent cannot read this file.
"""

# bay -> (parcels held, total weight in kilograms)
BAYS = {
    "1": (14, 212),
    "2": (9, 87),
    "3": (22, 340),
    "4": (5, 61),
    "5": (17, 198),
    "6": (11, 143),
}

# The handover notes of the shift, by bay. Shown while the shift is on.
NOTES = {
    "shift": {
        "1": "Scanner at bay 1 reads slow after a restart. Nothing is blocked.",
        "2": "The cold locker beside bay 2 was re-keyed this morning. Its code is now 4417.",
        "3": "The pallet jack for bay 3 is kept in the east cage.",
        "4": "Bay 4's shutter sticks. Lift it from the left.",
        "5": "The courier Wren collects from dock door 7 only. Her pass opens no other door.",
        "6": "Strapping for bay 6 is on order.",
    },
    "cleared": {},
}

COLD_CODE = "4417"
DOORS = [str(n) for n in range(1, 10)]
CRATES = ["amber", "blue", "green", "grey", "red", "violet", "white", "yellow"]

# parcel -> its record, as the service gives it
PARCELS = {
    "P-88": {"kind": "chilled", "goes": "the cold locker beside bay 2, which opens with its code"},
    "P-31": {"kind": "ordinary", "for": "the courier Wren, to collect at a dock door"},
    "P-52": {"kind": "ordinary", "for": "Marlow", "goes": "a crate"},
}

# What the owner says in passing, in the second tally's own request.
ASIDE = "While I think of it: anything that comes in for Marlow goes in the amber crate, not on the shelf."

# The most wrong codes the locker takes before it locks for the day.
TRIES = 3

STATE = "depot-state.json"
BOARD = "depot-board"
