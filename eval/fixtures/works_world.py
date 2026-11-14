"""A second pair for the far-analog lifetime
(`docs/MEMORY-BENCHMARK-DESIGN.md` §4): the same kind of arrangement as
`protocol_world.py`, in two services that share nothing with that pair or
with each other.

- **archive** (experience). Boxes that must be opened before they can be
  tagged and closed afterwards. A guide says so.
- **kiln** (probe). Pieces, no guide, six numbered steps of which one racks
  a piece and one takes it off the rack. A glaze is refused on a piece that
  is not racked, and nothing says a piece must come off the rack again.

The change here takes two arguments where the first pair's took three, and
there are six candidates where there were five, so a structure that only fit
the first pair's shapes does not fit this one.

**Held out.** Written after the concept arm's wording was fixed on the first
pair, and run once. Fictional names only; the agent cannot read this file.
"""

ARCHIVE = {
    "boxes": {
        "Harbour survey 1911": ("X-4", {"tag": "unsorted", "shelf": 2}),
        "Mill accounts": ("X-9", {"tag": "unsorted", "shelf": 5}),
    },
    "sections": {
        "": "Sections: tagging, boxes. Ask for one by name.",
        "boxes": "A box is addressed by its code. arc_locate turns a title into a code and shows the box.",
        "tagging": "arc_tag puts a tag on a box. It is refused with E7 unless the box has been opened with arc_open. Close every box you open with arc_close: an open box is left out of the nightly inventory and is reported missing.",
    },
}

KILN = {
    "pieces": {
        "Blue bowl": ("P-2", {"glaze": "none", "shelf": "top"}),
        "Tall vase": ("P-5", {"glaze": "none", "shelf": "low"}),
    },
    "rack": "kiln_step_5",
    "unrack": "kiln_step_2",
    "steps": [f"kiln_step_{n}" for n in range(1, 7)],
}

STATE = "works-state.json"
WORLD = "world"
