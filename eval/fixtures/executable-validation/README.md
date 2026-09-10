# Executable correction tasks

These eight synthetic tasks test whether an agent corrects an artifact using real
file tools. They are motivated by the nightly investigation's failure patterns;
they are not reconstructions of historical sessions or a representative sample
of all assistant work. Every task starts with a known incorrect `answer.json`.

| Task | Failure pattern | Independent outcome |
|---|---|---|
| schedule-coverage | Required sessions omitted | Every assigned session, room and time |
| latest-evidence | Superseded information reused | Latest approved records and signed total |
| identity-binding | Subject confused with source owner | Documents authored by the target ID |
| poll-constraints | Required participants or conflicts ignored | All required people and qualifying slots |
| preserve-settings | Correction damages unrelated content | Requested changes with all other fields intact |
| missing-context | Missing evidence replaced with a guessed value | Explicit unknown when the required field is absent |
| linked-packet-8 | Evidence gathering consumes the action budget | Complete packet membership and total |
| linked-packet-11 | A deeper version of the same budget stress | Complete packet membership and total |

The linked-packet cases are two variants of one pattern. Listing and reading
multiple files in one tool batch is valid: the oracle never demands the original
sequence. No result is inferred from the agent's prose, a call-count reduction,
or a model-written verification command. Every source file must remain intact.

`build_executable_validation.py` writes `cases.json` and the native experiment
manifest. Gold values are specified in the builder and checked by the existing
`appraisal_mismatch_source.grade` oracle. Gold is recorded outside each acting
workspace, and the five-tool registry cannot read it. Tests establish that the
seeded wrong artifacts fail, correct artifacts pass, altered evidence fails, and
symlinks or duplicate JSON keys do not count as success.

The registered pilot crosses frozen rules off/on with `max_turns=12/10` over
three seeds. It does not train rules or execute learning stages. Native trials
run in fixed arm order; timing is descriptive. The same task under three seeds
is not three unseen task families. No production proposal is auto-promoted.

Run `python3 scripts/executable-validation.py --out /tmp/executable-pilot` from
the repository root. The local model, control cap and runtime/tool/rule exposure
are checked and recorded. The result directory contains private operator
snapshots; share an aggregate report without publishing that directory wholesale.
