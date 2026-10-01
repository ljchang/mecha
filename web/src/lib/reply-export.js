// Copy and download for a chat reply (owner request, 2026-10-01): the reply
// as written — its Markdown, citations and all — never the rendered page,
// so what lands in a note or a file is what the model said.

// `mara-2026-10-01-1612.md`: who said it and when, in the owner's clock,
// with nothing a file system could choke on.
export function replyFilename(who, at = new Date()) {
  const slug =
    String(who ?? '')
      .toLowerCase()
      .normalize('NFKD')
      .replace(/[^\w\s-]/g, '')
      .trim()
      .replace(/[\s_]+/g, '-')
      .replace(/-+/g, '-')
      .slice(0, 40)
      .replace(/^-+|-+$/g, '') || 'reply';
  const p = (n) => String(n).padStart(2, '0');
  const d = at instanceof Date && !Number.isNaN(at.getTime()) ? at : new Date();
  return `${slug}-${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}.md`;
}

// Put `text` on the clipboard. The async API needs a secure page — the
// tailnet door is one — and a phone that refuses it still gets the old
// select-and-copy route, so a tap is never silently a no-op.
export async function copyText(text, nav = globalThis.navigator, doc = globalThis.document) {
  try {
    if (nav?.clipboard?.writeText) {
      await nav.clipboard.writeText(text);
      return true;
    }
  } catch {
    /* fall through to the selection route */
  }
  if (!doc?.body) return false;
  const area = doc.createElement('textarea');
  area.value = text;
  area.setAttribute('readonly', '');
  area.style.position = 'fixed';
  area.style.top = '0';
  area.style.left = '0';
  area.style.opacity = '0';
  doc.body.appendChild(area);
  area.select();
  let ok = false;
  try {
    ok = doc.execCommand?.('copy') ?? false;
  } catch {
    ok = false;
  }
  area.remove();
  return ok;
}

// Save `text` as a Markdown file named `name`, from a blob made here — the
// reply never goes back to the server to be downloaded.
export function downloadText(name, text, doc = globalThis.document, url = globalThis.URL) {
  const blob = new Blob([text], { type: 'text/markdown;charset=utf-8' });
  const href = url.createObjectURL(blob);
  const a = doc.createElement('a');
  a.href = href;
  a.download = name;
  a.rel = 'noopener';
  doc.body.appendChild(a);
  a.click();
  a.remove();
  // Revoked on the next turn, after the browser has taken the download.
  setTimeout(() => url.revokeObjectURL(href), 0);
}
