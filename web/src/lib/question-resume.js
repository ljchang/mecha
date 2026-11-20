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

/// The conversation an answer continued, as its session id, from
/// `POST /api/questions/answer`'s reply — or null when the resume was the
/// detached background one (or the reply named no usable session: then there
/// is no link to offer, and none is made up). The session id, never the key:
/// `#chat/<id>` resolves a session id and resumes it, handing back the live
/// key when serve already holds the conversation — the board's door into a
/// chat. A key in that slot is looked up as a session and 404s (review of #904).
export function answeredChat(reply) {
  if (reply?.resumed !== 'chat') return null;
  const session = typeof reply.session === 'string' ? reply.session.trim() : '';
  return session || null;
}

/// Where *Open chat* goes: the chat route, by session id.
export const chatHref = (session) => `#chat/${encodeURIComponent(session)}`;
