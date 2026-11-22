// The chat header's **auto** chip (`ASSISTANT-FOLLOW-THROUGH-DESIGN.md` F7):
// while on, the follow-ups this conversation proposes are accepted for the
// owner when it has read no outside content. Drafts are not covered — they
// always wait. The server owns the state (`POST /api/chat/{key}/auto`, the
// `auto` event, the transcript's `auto`); the chip only shows it.

import { isShown } from './features.js';

/** The chip's words, its tooltip, and its pressed state. */
export function autoChip(on) {
  return {
    label: on ? 'auto on' : 'auto off',
    pressed: on ? 'true' : 'false',
    title: on
      ? 'Follow-ups only: what this chat proposes is accepted for you when the conversation has read no outside content. Tainted ones, and every draft, still wait for you. Tap to turn off.'
      : 'Follow-ups only: tap to have what this chat proposes accepted for you when the conversation has read no outside content. Drafts always wait.',
  };
}

/**
 * Whether the chip is offered at all: never in a memory or incognito chat
 * (no recorded conversation, so nothing to propose into), and not with
 * `[features] followups` off — a switch for what does not run promises it.
 */
export function autoOffered({ memoryIntent, incognito, rows }) {
  return !memoryIntent && !incognito && isShown(rows, 'followups');
}

/** The state after a server word: the event's or the transcript's, off when absent. */
export function autoFrom(value) {
  return value === true;
}
