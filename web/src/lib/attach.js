// Files attached to a chat turn, in either chat — the assistant's
// (`Chat.svelte`) and a persona's (`Personas.svelte`). One copy of the drop
// rules, so a fix to what counts as a file lands in both.

// A drag of selected text or a link carries no files, and claiming it would
// swallow every in-page drag behind the overlay.
export const carriesFiles = (dt) => [...(dt?.types ?? [])].includes('Files');

// A dropped folder arrives as a File too — zero bytes, or a read error
// once fetch sends it — so it is told apart by its entry, never by size.
export function droppedFiles(dt) {
  const items = [...(dt?.items ?? [])].filter((it) => it.kind === 'file');
  if (!items.length) return { files: [...(dt?.files ?? [])], folders: [] };
  const files = [];
  const folders = [];
  for (const it of items) {
    const f = it.getAsFile();
    if (!f) continue;
    if (it.webkitGetAsEntry?.()?.isDirectory) folders.push(f.name);
    else files.push(f);
  }
  return { files, folders };
}

// The message a turn with attachments sends: each file named by its
// workspace path, so the model has something to hand a tool, and listed
// beside it (`attachments`) so the server can put each picture on the turn
// for a model that can see — the Slack door's pairing (REMOTE-SURFACE-DESIGN D6).
export function withAttachments(text, paths) {
  if (!paths.length) return text;
  const lines = paths.map((p) => `Attached file at ${p}`).join('\n');
  return text ? `${text}\n\n${lines}` : lines;
}
