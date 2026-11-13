"""The hidden rules and data of the opaque-service worlds
(`docs/MEMORY-BENCHMARK-DESIGN.md` §4, Tier B). Imported by the fixture
server, which enforces the rules, and by the task source, which grades
against them. An agent reaches neither file: both live in the checkout,
outside the trial's workspace.

Each world is one service with two things an agent can only learn by doing:

- **The token rule.** An export is addressed by a token built from the year,
  the period and the *class letter* of the account. The rule is stated only
  at the end of a chain of help topics, and the class comes from a lookup.
  A token for one account does not work for another, so what transfers is
  the rule, not a value.
- **The unit rule.** Amounts come back in minor units. Nothing says so until
  a check has been refused twice.

Two worlds share that structure and no wording, so a memory that helps on one
by keyword cannot help on the other. Fictional names only; this file is
public.
"""

import hashlib

WORLDS = {
    "ledger": {
        "server": "ledger",
        "noun": "account",
        "accounts": {
            "Harbor Supplies": ("ACC-0042", "K"),
            "Tidewater Freight": ("ACC-0117", "R"),
            "Juniper Print Works": ("ACC-0203", "M"),
        },
        "token": "{yy}:{pp:02d}#{cls}",
        "token_example": "26:05#K for the fifth period of FY26 on a class K account",
        "period_words": "fiscal period",
        "minor": 100,
        "minor_words": "minor units (hundredths)",
        "major_words": "dollars",
        "tools": {"help": "ledger_help", "lookup": "ledger_lookup", "export": "ledger_export", "check": "ledger_check"},
        "topics": {
            "": "Topics: export, accounts. Ask for one by name.",
            "accounts": "An account is addressed by its id. ledger_lookup turns a name into an id and a class letter.",
            "export": "ledger_export takes an account id and a period token. Token notation is under the topic 'periods'.",
            "periods": "Periods are fiscal, not calendar. The notation is under the topic 'fiscal'.",
            "fiscal": "A period token is YY:PP#C where YY is the fiscal year, PP the two-digit period and C the account's class letter from ledger_lookup, e.g. 26:05#K for the fifth period of FY26 on a class K account. Anything else is rejected with E17.",
        },
    },
    "scope": {
        "server": "scope",
        "noun": "instrument",
        "accounts": {
            "North Bench Spectrometer": ("INS-7", "Q"),
            "Annex Calorimeter": ("INS-12", "D"),
            "Basement Rheometer": ("INS-31", "V"),
        },
        "token": "{cls}/{yy}w{pp:02d}",
        "token_example": "Q/26w05 for the fifth run window of 2026 on a group Q instrument",
        "period_words": "run window",
        "minor": 1000,
        "minor_words": "millivolts",
        "major_words": "volts",
        "tools": {"help": "scope_manual", "lookup": "scope_find", "export": "scope_fetch", "check": "scope_verify"},
        "topics": {
            "": "Sections: fetch, instruments. Ask for one by name.",
            "instruments": "An instrument is addressed by its id. scope_find turns a name into an id and a group letter.",
            "fetch": "scope_fetch takes an instrument id and a window key. Key notation is under the section 'windows'.",
            "windows": "Windows are run windows, not calendar weeks. The notation is under the section 'keys'.",
            "keys": "A window key is G/YYwNN where G is the instrument's group letter from scope_find, YY the year and NN the two-digit window, e.g. Q/26w05 for the fifth run window of 2026 on a group Q instrument. Anything else is rejected with E17.",
        },
    },
}


def token(world, name, year, period):
    _, cls = WORLDS[world]["accounts"][name]
    return WORLDS[world]["token"].format(yy=year % 100, pp=period, cls=cls)


def rows(world, name, year, period):
    """The rows an export returns: deterministic, different for every
    account and period, so no answer can be carried over from another."""
    seed = hashlib.sha256(f"{world}|{name}|{year}|{period}".encode()).digest()
    count = 4 + seed[0] % 5
    return [{"line": i + 1, "amount": 1000 + (seed[i + 1] * 37) % 90000} for i in range(count)]


def total_minor(world, name, year, period):
    return sum(r["amount"] for r in rows(world, name, year, period))


def total_major(world, name, year, period):
    return total_minor(world, name, year, period) / WORLDS[world]["minor"]
