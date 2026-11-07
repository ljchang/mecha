// "Make a rule from this thread" (MAIL-RULES-DESIGN.md §8): a mail reader
// puts the open thread's headers here and navigates to #mail/rules, which
// takes them once and opens a new-rule form. Nothing is installed until the
// owner presses Install on that form; this only carries what was on screen.
export const ruleHandoff = $state({ thread: null });

/** The reader's half: what the row already holds — no read, no model. */
export function offerRuleFrom(row) {
  ruleHandoff.thread = row ? { from: row.from ?? '', subject: row.subject ?? '', account: row.account ?? '' } : null;
}

/** The rules page's half: the thread, at most once. */
export function takeRuleOffer() {
  const thread = ruleHandoff.thread;
  ruleHandoff.thread = null;
  return thread;
}
