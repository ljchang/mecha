// The owner's `/grant` and `/revoke`, typed into the web chat — the TUI's
// commands (`mecha_core::grant`), sent to `POST /api/chat/{key}/grants`
// instead of to the model. Parsed here, apart from the component, so the
// rules are tested as the page runs them (`test/grant-command.mjs`).
//
// `/grant` and `/grants` alone open the list. `/grant <path> [read|write]`
// asks for one; the access is the last word only when it is one, so a path
// with spaces survives. `/revoke <path>` ends this conversation's grants at
// or under it. Anything else is not a grant command and goes to the model as
// typed — including `/granted`, which is a word, not this command.

const ACCESS = { read: 'read', r: 'read', ro: 'read', write: 'write', w: 'write', rw: 'write' };

export function parseGrantCommand(text) {
  const m = /^\/(grants?|revoke)(?:\s+([\s\S]*))?$/.exec(text.trim());
  if (!m) return null;
  const [, name, rest = ''] = m;
  const arg = rest.trim();
  if (name === 'revoke') {
    return arg ? { action: 'revoke', path: arg } : { error: '/revoke needs a path — /grant lists what this chat holds' };
  }
  if (!arg) return { list: true };
  const split = /^([\s\S]*\S)\s+(\S+)$/.exec(arg);
  if (split && ACCESS[split[2].toLowerCase()]) {
    return { action: 'grant', path: split[1], access: ACCESS[split[2].toLowerCase()] };
  }
  return { action: 'grant', path: arg, access: 'read' };
}

// A grant from the global config is the file's to end, not this chat's.
export function revocable(grant) {
  return grant.source !== 'config';
}
