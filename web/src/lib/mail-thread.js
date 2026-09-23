// Shared by Mail.svelte (the phone) and MailDesk.svelte (the desktop), so the
// two readers of a thread cannot drift apart.
//
// Presentation-only parse of `mecha mail show`'s text: the leading
// `key:   value` block, then messages split on `--- ` separators. The
// CLI's text stays the one renderer of a thread — this shapes it for a
// phone and falls back to the raw text verbatim on any drift, so a
// format change degrades to yesterday's display, never to a wrong one.
export function parseThread(text) {
  const lines = text.split('\n');
  let i = 0;
  const header = [];
  while (i < lines.length && lines[i].trim() !== '') {
    const m = lines[i].match(/^([a-z]+):\s+(.*)$/);
    if (!m) return null;
    header.push([m[1], m[2]]);
    i++;
  }
  const chunks = lines
    .slice(i)
    .join('\n')
    .split(/\n(?=--- )/)
    .map((c) => c.trim())
    .filter(Boolean);
  const messages = [];
  for (const c of chunks) {
    if (!c.startsWith('--- ')) continue;
    const ls = c.split('\n');
    const meta = ls[0].replace(/^---\s*/, '');
    let j = 1;
    let subject = null;
    while (j < ls.length && ls[j].trim() !== '') {
      // The message-id line is addressed to the model, not the reader.
      if (ls[j].startsWith('Subject: ')) subject = ls[j].slice(9);
      j++;
    }
    const body = ls.slice(j).join('\n').trim();
    messages.push({ meta, subject, body });
  }
  return messages.length ? { header, messages } : null;
}
