// How a workflow's commitment reads on the Today page.
//
// `due_at` became optional in 1f-2 (ruling (b), 2026-09-25): a commitment an
// owner states through appraisal evidence names a party and an expectation
// but no date, and nothing may invent one. An absent date means "no deadline
// stated" — it is said in words, never rendered as a date. `new Date(undefined)`
// prints "Invalid Date", and a zero would print 1970: both are a dash read as
// a time.
export function commitmentLine(commitment, format = (at) => new Date(at).toLocaleString()) {
  if (!commitment) return '';
  const when = commitment.due_at ? `due ${format(commitment.due_at)}` : 'no deadline stated';
  return ` · ${when} · ${commitment.party}`;
}
