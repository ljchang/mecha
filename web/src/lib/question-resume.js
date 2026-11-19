// How an answered question goes back into its conversation
// (docs/ASSISTANT-FOLLOW-THROUGH-DESIGN.md §Unattended runs ask).
//
// A delegated task run's question resumes in the background, as it always
// has. A trigger's or a woken follow-up's resumes as a chat: `mecha serve`
// continues that conversation with the answer as the owner's own turn, and
// the reply carries the chat's key. Tasks stays the hub for background work
// (the owner's revision of A2): the page shows the answer as continuing,
// with the conversation one tap away, and never opens it by itself.

/// Will answering this question continue its conversation as a chat?
export const resumesInChat = (q) => q?.resume === 'chat';

/// The chat an answer continued, from `POST /api/questions/answer`'s reply,
/// or null when the resume was the detached background one (or the reply
/// named no usable key — then there is no link to offer, and none is made up).
export function answeredChat(reply) {
  if (reply?.resumed !== 'chat') return null;
  const key = typeof reply.key === 'string' ? reply.key.trim() : '';
  return key || null;
}

/// Where *Open chat* goes.
export const chatHref = (key) => `#chat/${encodeURIComponent(key)}`;
