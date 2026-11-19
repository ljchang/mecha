// A due follow-up waking its conversation arrives as a user turn written by
// the harness (`mecha_core::followup::render_wake`), never by the owner. The
// chat shows it as a note, not as the owner's bubble: "Follow-up due" and the
// words the conversation proposed. The stem is core's `WAKE_STEM`, and
// `the_page_knows_the_wake_stem` in Rust holds this copy to it.
export const WAKE_STEM = '(From the harness: a follow-up you proposed is due';

/** The proposed words when `text` is a wake turn, else null. */
export function followUpWake(text) {
  if (typeof text !== 'string' || !text.startsWith(WAKE_STEM)) return null;
  const m = text.slice(WAKE_STEM.length).match(/^ — "(.*?)", proposed /s);
  return m ? m[1] : '';
}
