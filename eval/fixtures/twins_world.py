"""The linked-change lifetime (`docs/MEMORY-BENCHMARK-DESIGN.md` §4, far
analog, second kind): a change that has to be made twice, on a thing and on
a second thing the first thing's own record names. Two pairs of services.

In each pair the **experience** face documents the rule and marks a change
whose twin disagrees with a code; the **probe** face has other tools, other
field names, no documentation, and answers every change the same way, so a
run that changes one thing only is told nothing. Every record also names
something that is not a twin.

- `signs` → `valves`: the development pair.
- `accounts` → `lamps`: **held out.** Written before the linked kind in
  `tool::concepts` was run on anything, and run once. Its change takes a
  number and its records have more fields.

- `berths` → `pumps`: **held out** for the reappraisal of failed tasks
  (`tool::episodes::Episodes::reappraising`). Written before that was run on
  anything, and run once. The experience face answers a change on one berth
  with a plain `ok` and no code: the only trace of the omission is a
  `state` field in the record, and its guide is the only thing that says
  what the field means.

Fictional names only; the agent cannot read this file.
"""

FACES = {
    "signs": {
        "kind": "string",
        "lookup": ("sg_find", "name", "Find a sign by its name."),
        "change": ("sg_text", "sign", "text", "Set the text a sign shows."),
        "docs": ("sg_manual", "topic", "Read the sign manual by topic. An empty topic lists the topics.", {
            "": "Topics: codes, signs. Ask for one by name.",
            "signs": "A sign is addressed by its code. sg_find turns a sign's name into its record.",
            "codes": "M3 after sg_text: the sign's mirror shows different text. Every sign stands opposite a mirror, named in its record under `mirror`. Put the same text on both with sg_text: a pair that disagrees is reported as a fault to the road office.",
        }),
        "code": "M3",
        "id": "sign", "value": "text", "twin": "mirror", "extra": [],
        "things": {
            "Bridge approach": ("S-3", "S-7", {"text": "Open", "side": "north", "post": "K-41"}),
            "Mill lane": ("S-12", "S-15", {"text": "Open", "side": "east", "post": "K-08"}),
        },
    },
    "valves": {
        "kind": "string",
        "lookup": ("vl_get", "tag", "Get a valve by its tag."),
        "change": ("vl_mode", "valve", "mode", "Set a valve's mode."),
        "docs": None, "code": None,
        "id": "valve", "value": "mode", "twin": "c2", "extra": ["vl_aux_a", "vl_aux_b", "vl_aux_c"],
        "things": {
            "North feed": ("V-2", "V-9", {"mode": "open", "zone": "Z-4", "kind": "gate"}),
            "Yard return": ("V-21", "V-30", {"mode": "open", "zone": "Z-1", "kind": "ball"}),
        },
    },
    "accounts": {
        "kind": "integer",
        "lookup": ("ac_open", "title", "Open an account's record by its title."),
        "change": ("ac_limit", "account", "limit", "Set an account's limit."),
        "docs": ("ac_help", "page", "Read the accounts help by page. An empty page lists the pages.", {
            "": "Pages: codes, accounts. Ask for one by name.",
            "accounts": "An account is addressed by its number. ac_open turns a title into the account's record.",
            "codes": "K7 after ac_limit: the account's contra account has a different limit. Every account is balanced by a contra account, named in its record under `contra`. Set the same limit on both with ac_limit: a pair with different limits is frozen at the month's close.",
        }),
        "code": "K7",
        "id": "account", "value": "limit", "twin": "contra", "extra": [],
        "things": {
            "Harbour dues": ("A-110", "A-910", {"limit": 500, "branch": "B-2"}),
            "Mill rents": ("A-140", "A-940", {"limit": 300, "branch": "B-5"}),
        },
    },
    "lamps": {
        "kind": "integer",
        "lookup": ("lp_show", "label", "Show a lamp by its label."),
        "change": ("lp_level", "lamp", "level", "Set a lamp's level."),
        "docs": None, "code": None,
        "id": "lamp", "value": "level", "twin": "peer", "extra": ["lp_test_1", "lp_test_2"],
        "things": {
            "Quay steps": ("L-4", "L-19", {"level": 2, "pole": "P-33", "circuit": "C-2", "lens": "clear"}),
            "Ferry gate": ("L-31", "L-36", {"level": 4, "pole": "P-07", "circuit": "C-5", "lens": "amber"}),
        },
    },
    "berths": {
        "kind": "string",
        "lookup": ("bt_find", "name", "Find a berth by its name."),
        "change": ("bt_class", "berth", "class", "Set the class of vessel a berth takes."),
        "docs": ("bt_guide", "section", "Read the berth guide by section. An empty section lists the sections.", {
            "": "Sections: state, berths. Ask for one by name.",
            "berths": "A berth is addressed by its code. bt_find turns a berth's name into its record.",
            "state": "A berth's record shows `state`: `paired` or `split`. Every berth shares a fender line with a partner berth, named in its record under `partner`, and both must take the same class of vessel. Set the same class on both with bt_class: a pair left `split` is closed by the harbour master.",
        }),
        "code": None, "status": ("state", "paired", "split"),
        "id": "berth", "value": "class", "twin": "partner", "extra": [],
        "things": {
            "Coal wharf": ("W-6", "W-8", {"class": "barge", "pier": "R-2"}),
            "Fish dock": ("W-15", "W-17", {"class": "barge", "pier": "R-5"}),
        },
    },
    "pumps": {
        "kind": "string",
        "lookup": ("pm_get", "tag", "Get a pump by its tag."),
        "change": ("pm_duty", "pump", "duty", "Set a pump's duty."),
        "docs": None, "code": None,
        "id": "pump", "value": "duty", "twin": "tied", "extra": ["pm_aux_1", "pm_aux_2"],
        "things": {
            "Bilge east": ("U-3", "U-11", {"duty": "idle", "sump": "M-7", "make": "vane"}),
            "Ballast aft": ("U-24", "U-29", {"duty": "idle", "sump": "M-2", "make": "gear"}),
        },
    },
}

STATE = "twins-state.json"
WORLD = "world"
