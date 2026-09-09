# Separate retirement drills

The earlier run passed: two attributed regressions retired the probationary bad
rule and left the bystander intact. After the harness-marker fix, the rerun exited
1 on its first pass: the model produced an unchanged both-pass pair, so the ledger
contained zero attributed regressions and both rules remained active. Its strict
elicitation/retirement assertion failed. This is not a passing retirement test.

Neither drill is part of the natural learning pilot. The failure is retained
without a score-seeking retry. The new marker filter did not change the retirement
algorithm, but this rerun does not demonstrate successful live retirement on the
final runtime. See each directory's log, ledger and final rule roster.
