"""A third pair for the far-analog lifetime
(`docs/MEMORY-BENCHMARK-DESIGN.md` §4): the same kind of arrangement as
`protocol_world.py` and `works_world.py`, in two services that share
nothing with those pairs or with each other.

- **greenhouse** (experience). Beds that must be isolated from the
  irrigation before their row count can be set, and reconnected afterwards.
  Its notes say so.
- **switchboard** (probe). Lines, no notes, seven lettered keys of which one
  parks a line and one restores it. A gain change is refused on a line that
  is not parked, and nothing says a line must be restored again.

The change here takes a number where the other pairs' changes took only
strings, and there are seven candidates, so a structure that only fit the
earlier pairs' shapes does not fit this one.

**Held out.** Written before the harness's cue for an owed step
(`tool::concepts::Concepts::cue`) was run on anything, and run once.
Fictional names only; the agent cannot read this file.
"""

GREENHOUSE = {
    "beds": {
        "North bench": ("B-3", {"rows": 2, "crop": "chard"}),
        "Long bed": ("B-8", {"rows": 3, "crop": "leek"}),
    },
    "pages": {
        "": "Pages: rows, beds. Ask for one by name.",
        "beds": "A bed is addressed by its code. gh_bed turns a bed's name into a code and shows the bed.",
        "rows": "gh_rows sets how many rows a bed is laid out in. It is refused with R2 unless the bed has been isolated with gh_isolate. Reconnect every bed you isolate with gh_reconnect: an isolated bed gets no water and the crop is lost within days.",
    },
}

SWITCHBOARD = {
    "lines": {
        "Harbour office": ("L-14", {"gain": 3, "room": "east"}),
        "Pump house": ("L-27", {"gain": 5, "room": "yard"}),
    },
    "park": "sw_key_f",
    "restore": "sw_key_c",
    "keys": [f"sw_key_{c}" for c in "abcdefg"],
}

STATE = "grounds-state.json"
WORLD = "world"
