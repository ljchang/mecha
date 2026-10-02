<script>
  import { onDestroy, tick, untrack } from 'svelte';
  import { apiFetch as fetch } from './api.js';
  import TomlForm from './TomlForm.svelte';
  import MdForm from './MdForm.svelte';
  import ModelChip from './ModelChip.svelte';
  import EditModal from './EditModal.svelte';
  import ChatProse from './ChatProse.svelte';
  import PersonaCall from './PersonaCall.svelte';
  import { features } from './features.svelte.js';
  import { isShown } from './features.js';
  import { composeEditMessage, maskName } from './image-edit.js';
  import { pictureOf, repeatedPictures, turnsWithoutPicture, downloadPicture } from './picture.js';
  import { carriesFiles, droppedFiles, withAttachments } from './attach.js';
  import { watchIdle, idleSpan } from './autolock.js';
  import { repairComments, changesOf } from './tomlform.js';
  import { isDirty as mdDirty } from './mdform.js';
  import {
    listUrl, personaUrl, chatUrl, relationshipLabel, emptyRun, applyEvent, settle, splitWaiting, proposalOrigin, isProposal,
    taintLabel, doseLine, authoringUrl, personaName, keptCharacter, OWNER_FILES, keptEdits,
    toolStatus, waitingLine, withWorking, unsavedFiles, lockWaits, fileUrl, uploadUrl, sourceLine, sourceState, fileKind,
    citeEntries, citeOpens, citedUrl, ownWords, toolRun, sourceFileUrl, chatHeadline,
    frameOf, frameStyle, dragFrame, MAX_FRAME_ZOOM,
  } from './persona.js';
  // The Personas tab (PERSONA-DESIGN.md §8; the owner's ruling of
  // 2026-09-29: a tab of its own, not a mode of the assistant's chat).
  //
  // Everything here goes through the persona door (`persona_chat.rs`) —
  // `persona.js` refuses to build any other URL — so nothing on this page can
  // read or write one of the assistant's conversations.
  //
  // The lock is the image library's: the same password, the same token, held
  // only in memory (the no-storage test holds the whole bundle to that) —
  // and the same autolock: the page locks itself after the owner's span with
  // no one touching it (`autolock.js`).

  let { initial = null } = $props();

  // The call with the open chat's persona (§11), started from the header.
  let caller = $state(null);

  let data = $state(null);
  let error = $state('');
  let token = $state(null);
  let password = $state('');
  let sheet = $state(false);
  let busy = $state(false);

  // The persona being looked at, its earlier chats, and the open chat.
  let chosen = $state(null);
  let history = $state([]);
  // Whether the list is still on its way, and why it could not be read: an
  // empty list is "no earlier chats", never a failed read in disguise.
  let historyLoading = $state(false);
  let historyNote = $state('');
  // The persona's files (§10): what it can read, and whether each is read yet.
  let sources = $state([]);
  let sourcesNote = $state('');
  let sourcesTimer = null;
  // Files on their way up, shown at once so a drop is answered before the
  // server is (the owner's ask, 2026-10-01).
  let uploading = $state([]);
  // A file dragged over the tiles.
  let dropping = $state(false);
  // A file opened from its tile: its line, a download, and its text once
  // read.
  let fileSheet = $state(null);
  // The goal field, behind a link: optional, and a distraction left open.
  let showGoal = $state(false);
  // The poll stops with the page, not with the last read (review of #459) —
  // and a load still in flight when it closes does not start it again.
  let gone = false;
  onDestroy(() => {
    gone = true;
    clearTimeout(sourcesTimer);
    // Leaving the tab ends the unlock: the page forgets its token, so the
    // server should too, rather than hold it to its idle span.
    stopIdle?.();
    if (token) revoke(token);
  });
  let key = $state(null);
  let run = $state(emptyRun());
  // The open chat's safety switches, as its transcript reports them (§12).
  let safety = $state(null);
  // The model the open chat's agent is bound to, for the chip (the same
  // picker as the assistant's chat: one model serves every surface), and
  // whether this chat has shown a crisis card — the support resources are
  // offered only after one (owner ruling, 2026-09-30).
  let chatModel = $state('');
  let crisisShown = $state(false);
  // The AI tag is what the `disclosure` switch shows (§12.1): on unless the
  // owner turned it off for this persona. The open chat's live switch wins
  // over the list's reading of the persona.
  const disclosed = $derived(((key && safety) || chosen?.safety)?.disclosure !== false);
  // A clock for the waiting line, ticking only while a run is live.
  let now = $state(Date.now());
  // On `running` alone, not the whole run: `run` is replaced on every
  // streamed word, and the tick would restart with each (review of #431).
  const running = $derived(run.running);
  $effect(() => {
    if (!running) return;
    now = Date.now();
    const tick = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(tick);
  });
  // Crisis cards the owner has closed, by id; a closed one leaves a link to
  // its resources rather than vanishing.
  let dismissed = $state(new Set());
  // The resources card opened from the chat's standing link.
  let showResources = $state(false);
  // Attached while a run was streaming: that run's end is re-read, because
  // what streamed before the stream opened is only on the server.
  let partial = false;
  // Authoring (§4.4): the form for a new persona, and the editor for one.
  let authoring = $state(null);
  let making = $state(null);
  let editing = $state(null);
  // A relationship or group being added from the form: { kind, name, text }.
  let adding = $state(null);
  // How many runs this page has seen end, so a `done` that overtakes
  // `attach`'s first read is noticed (`Chat.svelte`'s `doneSeq`).
  let doneSeq = 0;
  let refused = $state([]);
  let goal = $state('');
  let input = $state('');
  let source = null;
  let usedInitial = false;
  let scroller = $state(null);

  const personas = $derived(data?.personas ?? []);

  // A picture the persona drew (§8.6), under its row as in the assistant's
  // chat, with the same Edit button and modal. A locked persona's picture
  // carries the token and no link: opening it in a tab would write the token
  // into the browser's history, which outlives the unlock. An open persona's
  // needs neither, so its URL is safe to open full size.
  const repeats = $derived(repeatedPictures(run.entries));
  const noPicture = $derived(turnsWithoutPicture(run.entries, run.running));
  // Each answer's citations with the check made of each (§10.4).
  const cites = $derived(citeEntries(run.entries, run.citations));
  const pictureUrl = (path) => fileUrl(key, path, chosen?.locked ? token : null);

  // Download a generated picture: read and saved from a blob, so a locked
  // persona's picture leaves no address (with its unlock token) in the
  // browser's history — the reason it is not a link. Why one failed shows
  // under it.
  let pictureNote = $state(null); // { path, why }
  async function savePicture(path) {
    const k = key;
    const why = await downloadPicture(fetch, pictureUrl(path), path);
    // A chat left while the download ran keeps no note of it (review of #494).
    if (key !== k) return;
    // Only this picture's note: another's failure is not cleared by this one
    // succeeding (review of #494).
    if (why) pictureNote = { path, why };
    else if (pictureNote?.path === path) pictureNote = null;
  }

  // The Edit modal (EditModal.svelte): anything already typed becomes its
  // instruction. Not `editing`, which is the persona-file editor's.
  let imageEdit = $state(null);
  // A cited page open beside the chat (§10.4): the page as the chat read
  // it, the quote marked. Text drawn as text — never the file itself.
  let citedPage = $state(null);
  // Replies saved to the persona's files this visit, by the reply's text
  // (§10.5) — not its position, which a transcript re-read shifts when a
  // page-only notice sits between entries (the crisis cards' lesson, #418).
  let savedReplies = $state({});
  $effect(() => {
    key;
    untrack(() => (savedReplies = {}));
  });

  // What a drop may add: files, not folders, of the kinds the picker
  // offers — a dropped video is refused here, not after it has uploaded
  // (review of #479), the chat's own drop rule (`droppedFiles`).
  const ACCEPTED = /\.(pdf|png|jpe?g|webp|gif|md|markdown|txt)$/i;
  async function dropSources(dt) {
    const { files, folders } = droppedFiles(dt);
    const ok = files.filter((f) => ACCEPTED.test(f.name));
    const refused = files.length - ok.length + folders.length;
    await addSources(ok);
    // After the upload, which clears the line it starts with (review of #479).
    if (refused) {
      const why = `${refused} not added: drop PDFs, pictures, Markdown or text files${folders.length ? ', not folders' : ''}`;
      sourcesNote = sourcesNote ? `${sourcesNote} · ${why}` : why;
    }
  }

  // A file opened from its tile: a sheet with its line, a download, and —
  // once it has been read — its text. Never the PDF itself: a paper is
  // third-party content, and nothing but an image is served renderable.
  function openFile(s) {
    if (!chosen) return;
    fileSheet = { source: s, text: null, note: '', loading: false };
  }

  async function readFileText() {
    if (!chosen || !fileSheet) return;
    const want = fileSheet.source.name;
    fileSheet = { ...fileSheet, loading: true, note: '' };
    try {
      const res = await fetch(sourceFileUrl(chosen.name, want, 'text', token));
      if (!res.ok) throw new Error((await res.text()).trim());
      const { text } = await res.json();
      if (fileSheet?.source.name === want) fileSheet = { ...fileSheet, loading: false, text };
    } catch (e) {
      if (fileSheet?.source.name === want) fileSheet = { ...fileSheet, loading: false, note: String(e?.message ?? e) };
    }
  }

  // Save a reply into the persona's own files, on the owner's word: the
  // server takes only text this chat's persona wrote (`save_reply`).
  // Replies on their way to the server: a second click on the quiet link
  // must not write a second identical file (review of #475).
  let savingReplies = $state(new Set());

  async function saveReply(text) {
    if (!key || busy || savingReplies.has(text) || savedReplies[text]) return;
    const k = key;
    savingReplies = new Set([...savingReplies, text]);
    try {
      const res = await fetch(chatUrl(k, '/save', null), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ text, unlock: token ?? undefined }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const { name } = await res.json();
      if (key === k) {
        savedReplies = { ...savedReplies, [text]: name };
        loadSources();
      }
    } catch (e) {
      notice(`not saved: ${String(e?.message ?? e)}`);
    } finally {
      savingReplies.delete(text);
      savingReplies = new Set(savingReplies);
    }
  }
  // Which open answers: a slow first tap must not land under a later one's
  // header (review of #465), as `reread` counts with `readGen`.
  let citedGen = 0;

  async function openCited(check) {
    if (!key || !citeOpens(check)) return;
    const k = key;
    const gen = ++citedGen;
    citedPage = { file: check.file, page: check.found ?? check.cited, loading: true };
    try {
      const res = await fetch(citedUrl(k, check, chosen?.locked ? token : null));
      if (!res.ok) throw new Error((await res.text()).trim() || 'not found');
      const page = await res.json();
      if (key === k && citedPage && gen === citedGen) citedPage = { ...page, page: page.page ?? citedPage.page };
    } catch (e) {
      if (key === k && citedPage && gen === citedGen) citedPage = { ...citedPage, loading: false, error: String(e?.message ?? e) };
    }
  }
  function editImage(path) {
    imageEdit = { path, src: pictureUrl(path), initial: input.trim(), busy: false, error: null };
  }

  // The mask goes up into this chat's `inbox/` and is named in the message,
  // never attached — it is for `image_generate`, not for the persona to look
  // at (image-edit.js). The words then go out through `send`, like anything
  // typed, so a live run is steered just as a typed message steers it.
  async function sendEdit({ text, mask }) {
    const chatKey = key;
    const edit = imageEdit;
    if (!edit || !chatKey) return;
    edit.busy = true;
    edit.error = null;
    try {
      let maskPath = null;
      if (mask) {
        const res = await fetch(uploadUrl(chatKey, maskName(edit.path), token), { method: 'POST', body: mask });
        if (!res.ok) throw new Error((await res.text()).trim());
        maskPath = (await res.json()).path;
      }
      if (chatKey !== key) return;
      const message = composeEditMessage(edit.path, maskPath, text);
      if (!message) {
        edit.busy = false; // never a modal that no button can close
        return;
      }
      input = message;
      imageEdit = null;
      await send();
    } catch (err) {
      if (imageEdit === edit) {
        edit.busy = false;
        edit.error = `The mask could not be uploaded: ${err?.message ?? err}. Nothing was sent.`;
      }
    }
  }

  // Files for the next turn, as in the assistant's chat: each lands in this
  // chat's `inbox/` and is named in the message; a picture also rides on the
  // turn as pixels when the persona's model can see (`send_with`).
  let fileInput = $state(null);
  // A count, not a flag: a drop can land while a picked upload is going.
  let uploads = $state(0);
  let attachments = $state([]); // workspace-relative paths, announced on send
  let dragDepth = $state(0); // dragenter/leave fire at every child boundary
  // Only into an open chat, and not behind the edit modal: a file dropped
  // there would ride out unseen on the modal's own send (review of #429).
  const canDrop = $derived(!!key && !imageEdit);

  const notice = (text) => (run = { ...run, entries: [...run.entries, { kind: 'notice', text }] });

  async function uploadFiles(files) {
    const chatKey = key;
    for (const f of files) {
      uploads += 1;
      try {
        const res = await fetch(uploadUrl(chatKey, f.name, token), { method: 'POST', body: f });
        if (!res.ok) throw new Error((await res.text()).trim());
        const data = await res.json();
        // A switch mid-upload must not announce this chat's file in the next.
        if (chatKey !== key) return;
        attachments.push(data.path);
      } catch (err) {
        if (chatKey !== key) return;
        notice(`upload failed: ${err?.message ?? err}`);
      } finally {
        uploads -= 1;
      }
    }
  }

  function uploadPicked(e) {
    const files = [...(e.target.files ?? [])];
    e.target.value = '';
    uploadFiles(files);
  }

  // On the window, as the assistant's chat does: a file dropped anywhere the
  // page does not claim is *navigated to* by the browser, which throws the
  // open chat away. Claimed on every screen of this tab, attached only into
  // an open chat.
  function onDragEnter(e) {
    if (!carriesFiles(e.dataTransfer)) return;
    e.preventDefault();
    dragDepth += 1;
  }
  function onDragOver(e) {
    if (!carriesFiles(e.dataTransfer)) return;
    // The Files tiles claimed it: their drop, not the chat's (review of #479).
    if (e.defaultPrevented) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = canDrop ? 'copy' : 'none';
  }
  function onDragLeave(e) {
    if (!carriesFiles(e.dataTransfer)) return;
    dragDepth = Math.max(0, dragDepth - 1);
  }
  function onDrop(e) {
    if (!carriesFiles(e.dataTransfer)) return;
    // Counted down whoever takes the drop, so the overlay never sticks.
    dragDepth = 0;
    if (e.defaultPrevented) return;
    e.preventDefault();
    if (!canDrop) return;
    const { files, folders } = droppedFiles(e.dataTransfer);
    for (const name of folders) notice(`not attached: ${name} is a folder — drop the files inside it`);
    uploadFiles(files);
  }

  async function load() {
    error = '';
    try {
      const res = await fetch(listUrl(token));
      if (!res.ok) throw new Error((await res.text()).trim());
      data = await res.json();
      // A token the server no longer honours has lapsed — its idle span, or
      // a restart, which forgets every token.
      if (token && !data.unlocked) dropToken();
      // One that left the list (hidden by that lapse, or removed) closes
      // whole, its chat stream and all, not just its heading.
      if (chosen) {
        const again = personas.find((p) => p.name === chosen.name);
        if (again) chosen = again;
        else toList();
      }
      // A deep link (`#personas/mara`) opens that persona, earlier chats and
      // all — once: a later reload (a lock toggle) must not re-enter it.
      // `#personas/+new` opens the form for a new persona — `+`, because
      // `new` could be a persona's own name (review of #420).
      if (!chosen && initial === '+new' && !usedInitial) {
        usedInitial = true;
        await startMaking();
      }
      if (!chosen && initial && !usedInitial) {
        usedInitial = true;
        chosen = personas.find((p) => p.name === initial) ?? null;
        if (chosen) {
          loadReview();
          await loadHistory();
        }
      }
      // No reset here: `error` was cleared at the top, and anything set
      // since — a failed `+new` deep link — is the one worth showing.
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  // Once, on mount. `load` reads `token` and `chosen` before its first
  // await, so called bare here it made them this effect's dependencies: every
  // lock toggle re-ran it, and the teardown closed the open chat's stream
  // while `key` stayed set — a chat frozen without a word (review of #415).
  $effect(() => {
    untrack(() => load());
    return () => close();
  });

  async function unlock() {
    busy = true;
    try {
      const res = await fetch('/api/library/unlock', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ password }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const granted = await res.json();
      token = granted.token;
      armIdle(granted.idle_secs);
      sheet = false;
      await load();
      await rereadAuthoring();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      password = '';
      busy = false;
    }
  }

  // The autolock: the owner's span with no one touching the page locks it,
  // and a return to the page asks whether the token outlived a restart.
  let stopIdle = null;
  function armIdle(secs) {
    stopIdle?.();
    stopIdle = watchIdle({ idleMs: idleSpan(secs) * 1000, onIdle: relock, onReturn: load });
  }

  function dropToken() {
    token = null;
    stopIdle?.();
    stopIdle = null;
  }

  function revoke(t) {
    fetch('/api/library/relock', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ token: t }),
    }).catch(() => {});
  }

  async function relock() {
    const t = token;
    dropToken();
    // A framing sheet over a persona the lock may hide goes with it.
    framing = null;
    // Off the screen now, before the grid's round trip, not after it.
    if (making && authoring) authoring = { ...authoring, characters: [] };
    // A locked persona's chat closes with the lock: the lock hides (§8.3).
    if (chosen?.locked) toList();
    await load();
    await rereadAuthoring();
    if (t) revoke(t);
  }

  function close() {
    source?.close();
    source = null;
  }

  // The composer grows with what is typed, up to its max-height.
  function grow(node) {
    // Empty keeps the stylesheet's one-line height: measured at mount, before
    // the flex row has given it a width, the placeholder wraps and reads tall.
    const fit = () => {
      node.style.height = '';
      if (!node.value) return;
      node.style.height = 'auto';
      node.style.height = `${Math.min(node.scrollHeight, 160)}px`;
    };
    requestAnimationFrame(fit);
    node.addEventListener('input', fit);
    // `update` runs when `input` changes — a send clears it — but whether
    // before or after `bind:value` writes the element is Svelte's ordering to
    // decide; a frame later the value is settled either way (review of #431).
    return { update: () => requestAnimationFrame(fit), destroy: () => node.removeEventListener('input', fit) };
  }

  // One step back at a time: out of a chat or the editor to the persona,
  // and from the persona to the list. The chat's "Done" button was this.
  // The lock never steps: `toList` hides everything at once (review of #431).
  function back() {
    if (key) {
      close();
      key = null;
      imageEdit = null;
      pictureNote = null;
      attachments = [];
      run = emptyRun();
      // The chat's switches leave with it: a persona page reads its own
      // (review of #431 — the last chat's `disclosure` decided the tag).
      safety = null;
      loadHistory();
      return;
    }
    if (editing) {
      editing = null;
      return;
    }
    toList();
  }

  function toList() {
    close();
    safety = null;
    making = null;
    editing = null;
    chosen = null;
    key = null;
    imageEdit = null;
    pictureNote = null;
    // A sheet over a persona that has gone — a relock lands here — must go
    // with it, or it renders without one (review of #479) — the framing
    // sheet too, or the next persona opens it with this one's frame and
    // Save writes it there (review of #491).
    fileSheet = null;
    framing = null;
    citedPage = null;
    attachments = [];
    run = emptyRun();
    history = [];
    refused = [];
  }

  // ─── A proposal, read and approved here (§4.4; the owner's rulings of
  // 2026-10-01) ────────────────────────────────────────────────────────
  // A persona the main chat proposed waits on this page, never in that chat:
  // approval beside the model's own pitch is where reading cold is hardest.
  // What is read carries the server's signature of it, and approving sends
  // the signature back, so what is approved is what was shown — a revision
  // landing between the read and the tap is refused and read again.
  let review = $state(null);
  let reviewNote = $state('');
  let rejectArmed = $state(false);
  let reviewGen = 0;
  const waitingSplit = $derived(splitWaiting(personas));

  async function loadReview() {
    const name = chosen?.name;
    const gen = ++reviewGen;
    review = null;
    reviewNote = '';
    rejectArmed = false;
    if (!name || !isProposal(chosen)) return;
    try {
      const res = await fetch(personaUrl(name, '/review', token));
      if (gen !== reviewGen) return;
      if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      const read = await res.json();
      if (gen === reviewGen) review = read;
    } catch (e) {
      if (gen === reviewGen) reviewNote = String(e?.message ?? e);
    }
  }

  async function approveProposal() {
    if (!review || busy) return;
    busy = true;
    reviewNote = '';
    try {
      const res = await fetch(personaUrl(chosen.name, '/approve', null), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          shown: review.shown,
          // The portrait with it, in the same tap, only while it waits too.
          character_shown: review.character?.waiting ? review.character.shown : undefined,
          unlock: token ?? undefined,
        }),
      });
      if (!res.ok) {
        reviewNote = (await res.text()).trim();
        // Changed since it was read: show what it says now.
        if (res.status === 409) {
          const why = reviewNote;
          await loadReview();
          reviewNote = why;
        }
        return;
      }
      review = null;
      await load();
      await loadHistory();
    } catch (e) {
      reviewNote = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  async function rejectProposal() {
    if (!rejectArmed) {
      rejectArmed = true;
      return;
    }
    busy = true;
    try {
      const res = await fetch(personaUrl(chosen.name, '/reject', null), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ unlock: token ?? undefined }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      toList();
      await load();
    } catch (e) {
      reviewNote = String(e?.message ?? e);
    } finally {
      busy = false;
      rejectArmed = false;
    }
  }

  async function choose(p) {
    showGoal = false;
    close();
    framing = null;
    // Another persona's files must not draw under this one's heading while
    // its own load (review of #459).
    sources = [];
    sourcesNote = '';
    history = [];
    historyNote = '';
    chosen = p;
    key = null;
    run = emptyRun();
    refused = [];
    goal = '';
    loadReview();
    await loadHistory();
  }

  // Numbered, as the transcript's reads are: a slow answer for the last
  // persona never lands under this one. A failure keeps what was listed and
  // says so — it used to read as "no earlier chats", which is what a token
  // lapsed by a server restart looked like (the page then re-asks the list,
  // which drops the token and closes a persona it hid).
  let historyGen = 0;
  async function loadHistory() {
    if (!chosen) return;
    loadSources();
    const name = chosen.name;
    const gen = ++historyGen;
    historyLoading = true;
    let why = '';
    try {
      const res = await fetch(personaUrl(name, '/chats', token));
      if (gen !== historyGen) return;
      if (res.ok) {
        // The body is a second wait: a newer read may have started in it.
        const { chats } = await res.json();
        if (gen !== historyGen) return;
        history = chats;
      } else {
        why = (await res.text()).trim() || `HTTP ${res.status}`;
        if (token) await load();
      }
    } catch (e) {
      why = String(e?.message ?? e);
    } finally {
      if (gen === historyGen) {
        historyLoading = false;
        historyNote = why && chosen?.name === name ? `earlier chats could not be read: ${why}` : '';
      }
    }
  }

  // Read again while anything is still being read, so "reading…" turns to
  // "ready" without a reload.
  async function loadSources() {
    clearTimeout(sourcesTimer);
    if (!chosen) return;
    const name = chosen.name;
    try {
      const res = await fetch(personaUrl(name, '/sources', token));
      if (chosen?.name !== name) return;
      sources = res.ok ? (await res.json()).sources : [];
    } catch {
      sources = [];
    }
    if (!gone && sources.some((s) => s.processing)) sourcesTimer = setTimeout(loadSources, 3000);
  }

  async function addSources(files) {
    sourcesNote = '';
    if (!chosen || !files?.length) return;
    busy = true;
    const failed = [];
    uploading = files.map((f) => f.name);
    try {
      for (const f of files) {
        const url = personaUrl(chosen.name, '/sources', token);
        const res = await fetch(`${url}${url.includes('?') ? '&' : '?'}name=${encodeURIComponent(f.name)}`, {
          method: 'POST',
          body: f,
        });
        // Every failure, not the last: three of five refused says three.
        if (!res.ok) failed.push(`${f.name}: ${(await res.text()).trim()}`);
        // This one, not every file of its name (review of #479).
        const at = uploading.indexOf(f.name);
        uploading = uploading.filter((_, k) => k !== at);
      }
      sourcesNote = failed.join(' · ');
    } catch (e) {
      sourcesNote = [...failed, String(e?.message ?? e)].join(' · ');
    } finally {
      uploading = [];
      busy = false;
      await loadSources();
    }
  }

  async function removeSource(file) {
    if (!chosen || busy) return;
    sourcesNote = '';
    busy = true;
    try {
      const res = await fetch(personaUrl(chosen.name, '/sources/remove', null), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ file, unlock: token ?? undefined }),
      });
      if (!res.ok) sourcesNote = (await res.text()).trim();
    } catch (e) {
      sourcesNote = String(e?.message ?? e);
    } finally {
      busy = false;
    }
    await loadSources();
  }

  async function scrollDown() {
    await tick();
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }

  // Read the transcript as the server holds it now, keeping what only the
  // page holds (`settle`). Reads are numbered so a slow one never lands over
  // a newer one.
  let readGen = 0;
  async function reread(k) {
    const gen = ++readGen;
    const res = await fetch(chatUrl(k, '', token));
    if (!res.ok) throw new Error((await res.text()).trim());
    const t = await res.json();
    if (key !== k || gen !== readGen) return;
    const entries = t.running ? withWorking(settle(t.entries, run), t.working) : settle(t.entries, run);
    run = { ...emptyRun(entries, t.taint ?? null, t.citations ?? []), running: !!t.running };
    safety = t.safety ?? null;
    chatModel = t.model ?? '';
    crisisShown = !!t.crisis_shown;
    scrollDown();
    return t;
  }

  // The stream opens *before* the read, and a finished run is read again:
  // a page attached mid-run cannot rebuild the part of the answer that
  // streamed before it subscribed, and without the re-read that turn would
  // read as a complete but truncated reply (review of #415; `Chat.svelte`
  // argues the same hazard).
  async function attach(k) {
    close();
    key = k;
    run = emptyRun();
    // A modal over the last chat's picture must not send into this one, and
    // the last chat's files are paths in another jail.
    imageEdit = null;
    pictureNote = null;
    attachments = [];
    // Or the previous chat's resources show for a round trip (review of #418).
    safety = null;
    crisisShown = false;
    chatModel = '';
    dismissed = new Set();
    showResources = false;
    partial = false;
    const s = new EventSource(chatUrl(k, '/events', token));
    source = s;
    s.onmessage = (m) => {
      let ev;
      try {
        ev = JSON.parse(m.data);
      } catch {
        return; /* a malformed event is dropped, never drawn */
      }
      run = applyEvent(run, ev);
      scrollDown();
      if (ev.type === 'done') {
        doneSeq += 1;
        // Only a run that finished is re-read: a failed one was rolled back
        // on the server, and the page's own record of it — the message and
        // why it failed — is the one worth keeping on screen.
        // And only when this page joined it midway: otherwise the stream
        // carried the whole turn (review of #415).
        if (ev.ok && partial) {
          partial = false;
          reread(k).catch((e) => (error = String(e?.message ?? e)));
        }
        loadHistory();
      }
    };
    // A stream the server ended (a relock, a restart) is said, not frozen.
    // One the browser is re-opening lost whatever was sent in the gap — the
    // server keeps no replay — so the run it rejoins is read again at its
    // end, as a late join is (review of #418).
    s.onerror = () => {
      if (source !== s) return;
      if (s.readyState === 2) error = 'this chat stopped updating — open it again';
      else partial = true;
    };
    try {
      const seen = doneSeq;
      const t = await reread(k);
      partial = !!t?.running;
      // The run ended while that read was in flight: its `done` found
      // nothing to re-read and the read says "running" — read once more.
      if (partial && doneSeq !== seen) {
        partial = false;
        await reread(k);
      }
    } catch (e) {
      close();
      key = null;
      error = String(e?.message ?? e);
    }
  }

  // ─── Authoring ───────────────────────────────────────────────────────

  // Every read of the form's lists is numbered, as `reread` numbers the
  // transcript's: the lock button is live while one is in flight, and an
  // unlocked answer landing after a relock's would put locked characters
  // back in the portrait list of a locked page (review of #425).
  let authoringGen = 0;

  async function startMaking() {
    error = '';
    const gen = ++authoringGen;
    const issued = token;
    try {
      const res = await fetch(authoringUrl(token));
      if (!res.ok) throw new Error((await res.text()).trim());
      const lists = await res.json();
      if (gen !== authoringGen) return;
      // The lock moved while the form was opening, and with no form open
      // then, nothing re-read the lists for it. An answer read under the
      // other lock is never shown, not even until the re-read lands.
      authoring = issued === token ? lists : null;
      making = { name: '', display: '', relationships: [], character: '', groups: [], locked: false };
    } catch (e) {
      error = String(e?.message ?? e);
      return;
    }
    if (issued !== token) await rereadAuthoring();
  }

  // The form's lists follow the lock: `authoring` was read when the form
  // opened, so an unlock after that left locked characters out of the
  // portrait list, and a relock left them in.
  async function rereadAuthoring() {
    if (!making) return;
    const gen = ++authoringGen;
    // Reading under the lock: the list on screen may name locked characters,
    // so it goes now, not when the answer lands — the rule `startMaking`
    // keeps for the same race (review of #425). The choice itself waits for
    // the answer, so a relock never costs an unlocked pick.
    if (!token && authoring) authoring = { ...authoring, characters: [] };
    try {
      const res = await fetch(authoringUrl(token));
      if (!res.ok) throw new Error((await res.text()).trim());
      const lists = await res.json();
      // Cancelled, or overtaken by a newer read, while this one was in flight.
      if (!making || gen !== authoringGen) return;
      authoring = lists;
      making.character = keptCharacter(making.character, authoring.characters);
    } catch (e) {
      error = String(e?.message ?? e);
      // No answer: keep only a choice the list on screen still offers, which
      // after a relock is none — never a locked pick the page now hides.
      if (making && gen === authoringGen) {
        making.character = keptCharacter(making.character, authoring?.characters);
      }
    }
  }

  const TEMPLATE = (name) =>
    `# ${name}\n\n<!-- How someone in this relationship behaves. Everything outside\n     comments like this one is read into every chat with a persona that\n     names it. -->\n\n- \n`;

  function startAdding(kind) {
    adding = { kind, name: '', text: '' };
  }

  // Add the relationship or group, then pick it for the persona being made.
  async function addNew() {
    const name = personaName(adding.name);
    if (!name) {
      error = 'a name is lowercase letters, digits, - and _';
      return;
    }
    busy = true;
    error = '';
    try {
      const relationship = adding.kind === 'relationship';
      const res = await fetch(relationship ? '/api/personas/relationships' : '/api/personas/groups', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(
          relationship
            ? { name, text: adding.text.trim() ? adding.text : TEMPLATE(adding.name.trim()) }
            : { name, description: adding.text.trim() || undefined },
        ),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      // The one reader of the lists once the form is open, so the new name
      // arrives under the same numbering and portrait check a lock change
      // uses — its own read took a newer number than a relock's and skipped
      // the check (review of #425).
      await rereadAuthoring();
      // Cancelled while the add was in flight: no form left to pick it for.
      if (!making) return;
      if (relationship) making.relationships = [...making.relationships, name];
      else making.groups = [...making.groups, name];
      adding = null;
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  function toggle(list, item) {
    return list.includes(item) ? list.filter((x) => x !== item) : [...list, item];
  }

  async function make() {
    const name = personaName(making.name);
    if (!name) {
      error = 'a name is lowercase letters, digits, - and _, and not one of the store’s own folders';
      return;
    }
    busy = true;
    error = '';
    try {
      const res = await fetch('/api/personas', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ ...making, name, display: making.display.trim() || undefined, character: making.character || undefined }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      making = null;
      await load();
      const made = personas.find((p) => p.name === name);
      if (made) {
        await choose(made);
        await openEditor('identity');
        // A persona made with a portrait is framed first (owner, 2026-10-01).
        if (chosen?.portrait) openFraming();
      }
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  async function openEditor(file = 'identity') {
    error = '';
    try {
      const res = await fetch(personaUrl(chosen.name, '/files', token));
      if (!res.ok) throw new Error((await res.text()).trim());
      const files = await res.json();
      editing = { files, file, text: files[file].text, base: files[file].digest, saved: null };
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  function pickFile(file) {
    // The other files keep what was typed in them until saved or closed —
    // the text draft only from a tab in text mode, where `editing.text` is
    // what was typed (review of #430).
    if (!asForm) editing.files[editing.file].draft = editing.text;
    editing.file = file;
    editing.text = editing.files[file].draft ?? editing.files[file].text;
    editing.base = editing.files[file].digest;
    editing.saved = null;
  }

  // Every owner file is edited as a form — persona.toml as typed fields
  // (`TomlForm`), the Markdown as a title and sections (`MdForm`) — unless
  // the owner chose text, or the server could not read it into one.
  // "Edit as text" is per file and stays the escape hatch.
  const current = $derived(editing ? editing.files[editing.file] : null);
  const hasForm = $derived(Boolean(current?.form?.form || current?.form?.doc));
  const asForm = $derived(hasForm && !current.asText);
  const textDirty = $derived(editing ? editing.text !== current.text : false);
  const formDirty = $derived.by(() => {
    const f = current?.form;
    if (!f || !current.formDraft) return false;
    if (f.form) return Object.keys(changesOf(f.form, f.values, current.formDraft)).length > 0;
    return mdDirty(f.doc, current.formDraft);
  });

  // Locking without the unlock closes the editor, so it waits for unsaved
  // changes in any tab, which the hint names (`persona.js`).
  const waitingTabs = $derived(lockWaits({ chosen, token, editing }) ? unsavedFiles(editing) : []);
  const lockWaitsNow = $derived(waitingTabs.length > 0);

  // Switch between the form and the text. Refused while the side being left
  // holds unsaved changes — each saves against the file as it stands, so
  // one would silently lose the other's edits.
  function setAsText(on) {
    current.asText = on;
    // Text mode picks up the text draft this tab already held, if any —
    // what was typed survives (review of #420) — else the file as saved.
    if (on) editing.text = current.draft ?? current.text;
    editing.saved = null;
  }

  // What the page labels each Markdown form with.
  const MD_LABELS = {
    identity: {
      title: 'Name',
      body: 'Text before the sections',
      fixed: { Core: 'Who they are at heart — never changed on its own, and re-read to them in long chats.' },
    },
    motivation: { title: 'Title', body: 'What they want and value, in your words' },
  };

  const saveText = () => saveFile({
    // A phone's "smart" dashes break a Markdown comment; put it back.
    text: editing.file === 'settings' ? editing.text : repairComments(editing.text),
  });
  const saveForm = (changes) => saveFile({ changes });
  const saveDoc = (doc) => saveFile({ doc });

  async function saveFile(payload) {
    busy = true;
    error = '';
    try {
      const res = await fetch(personaUrl(chosen.name, '/files', null), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ file: editing.file, ...payload, base: editing.base, unlock: token ?? undefined }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const saved = await res.json();
      const file = editing.file;
      // What was typed in the other files — as text or in a form — survives
      // this save (reviews of #420 and #430).
      const kept = keptEdits(editing.files, file);
      const portrait = chosen?.portrait;
      await load();
      // A new portrait — a character named in the settings — is framed
      // straight away (owner, 2026-10-01).
      if (chosen?.portrait && chosen.portrait !== portrait) openFraming();
      await openEditor(file);
      for (const [f, v] of Object.entries(kept)) Object.assign(editing.files[f], v);
      editing.saved = `saved — now v${saved.version}; new chats use it`;
      // A save can succeed and still leave something to fix — a group not
      // declared, a Core left empty: said here, not dropped.
      editing.problems = saved.problems ?? [];
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  // ─── The avatar's framing ────────────────────────────────────────────
  // A sheet over the persona page: drag the picture in its circle, zoom
  // with the slider. Nothing is sent until Save; a frame is display only
  // (`State::frame`), so it is no new version and reaches no prompt.
  // { frame, aspect, from: { x, y, at } | null } — `aspect` is the picture's
  // width / height, read when it loads: a drag pans only what is hidden.
  let framing = $state(null);
  const FRAME_SIZE = 220;

  // A new portrait arrives unframed — the server clears the old picture's
  // crop when the character changes (`persona::write_owner_file`) — so the
  // sheet opens on the default and Cancel leaves the default (review of #491).
  function openFraming() {
    framing = { frame: frameOf(chosen.frame), aspect: 1, from: null };
  }

  function frameDown(e) {
    e.currentTarget.setPointerCapture?.(e.pointerId);
    framing.from = { x: e.clientX, y: e.clientY, at: framing.frame };
  }

  function frameMove(e) {
    if (!framing?.from) return;
    const { x, y, at } = framing.from;
    framing.frame = dragFrame(at, e.clientX - x, e.clientY - y, FRAME_SIZE, framing.aspect);
  }

  function frameUp() {
    if (framing) framing.from = null;
  }

  // `null` returns it to the page's default framing, never a stored copy of it.
  async function saveFrame(frame) {
    busy = true;
    try {
      const res = await fetch(personaUrl(chosen.name, '/frame', null), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ frame, unlock: token ?? undefined }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      framing = null;
      await load();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  async function setLocked(locked) {
    busy = true;
    try {
      const res = await fetch(personaUrl(chosen.name, '/lock', null), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ locked, unlock: token ?? undefined }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      // Locked with no unlock in hand, it is hidden now: `load` takes the
      // page back to the list itself.
      await load();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  async function start() {
    busy = true;
    error = '';
    try {
      const res = await fetch(personaUrl(chosen.name, '/chats', null), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ unlock: token ?? undefined, goal: goal.trim() || undefined }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const opened = await res.json();
      refused = opened.refused ?? [];
      goal = '';
      await attach(opened.key);
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  async function resume(id) {
    busy = true;
    error = '';
    try {
      const res = await fetch(personaUrl(chosen.name, '/resume', null), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ id, unlock: token ?? undefined }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const opened = await res.json();
      refused = opened.refused ?? [];
      await attach(opened.key);
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  async function send() {
    const typed = input.trim();
    const attached = [...attachments];
    const text = withAttachments(typed, attached);
    if (!text || !key) return;
    input = '';
    attachments = [];
    try {
      const res = await fetch(chatUrl(key, '/send'), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ text, attachments: attached, unlock: token ?? undefined }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      // Pictures the persona was not shown, said here since the chips are
      // gone — the assistant's chat's wording, for the same three reasons.
      if (data.started && data.pictures_not_shown > 0) {
        notice(
          data.model_sees
            ? `${data.pictures_not_shown} picture(s) went in by name only — ${chosen.display} was not shown them.`
            : 'This model cannot see images, so the picture(s) went in by name only.',
        );
      }
      // A steer carries text only, so a picture sent into a live run is
      // named and not shown.
      if (data.steered && attached.some((p) => /\.(png|jpe?g|gif|webp)$/i.test(p))) {
        notice(`${chosen.display} was answering, so the picture went in by name only — not shown.`);
      }
    } catch (e) {
      // Nothing was sent: the words and the files come back to the composer.
      input = typed;
      attachments = attached;
      error = String(e?.message ?? e);
    }
  }

  async function stop() {
    if (!key) return;
    await fetch(chatUrl(key, '/cancel'), {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ unlock: token ?? undefined }),
    }).catch(() => {});
  }

  function onKey(e) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  }

  const clock = (iso) => {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
  };

  const when = (iso) => {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? '' : d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  };
</script>

{#snippet avatar(p, size)}
  <span class="avatar" style="width:{size}px;height:{size}px;font-size:{Math.round(size * 0.42)}px">
    {#if p.portrait}<img src={p.portrait} alt="" style={frameStyle(p.frame)} />{:else}{p.display.slice(0, 1).toUpperCase()}{/if}
  </span>
{/snippet}

<svelte:window
  ondragenter={onDragEnter}
  ondragover={onDragOver}
  ondragleave={onDragLeave}
  ondrop={onDrop}
  onkeydown={(e) => {
    // Escape closes the framing sheet, as Cancel does — at the window, since
    // the sheet can open unasked with focus left in the editor (review of
    // #491).
    if (e.key === 'Escape' && framing) framing = null;
  }}
/>

<div class="page">
  {#if dragDepth > 0 && canDrop}
    <div class="drop-overlay" aria-hidden="true">
      <div class="drop-card">
        <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="var(--accent-400)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.5l-8.2 8.2a5.5 5.5 0 01-7.8-7.8L13.6 4.3a3.7 3.7 0 015.2 5.2l-8.4 8.4a1.85 1.85 0 01-2.6-2.6l7.8-7.8" /></svg>
        <span>drop to attach — files land in this chat's inbox/</span>
      </div>
    </div>
  {/if}
  <header class="head">
    {#if chosen}
      <button class="backbtn" aria-label={key ? `leave the chat with ${chosen.display}` : editing ? 'close the editor' : 'all personas'} onclick={back}>
        <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M15 5l-7 7 7 7" /></svg>
      </button>
      <!-- On the persona's own page the hero says who; the header only
           needs the way back. -->
      {#if key || editing}
        {@render avatar(chosen, 32)}
        <!-- The name and the AI tag, nothing more (owner, 2026-09-30).
             Disclosure is the harness's, not the persona's (§12.1). -->
        <div class="who">
          <span class="pname">{chosen.display}{#if disclosed}<span class="ai">AI</span>{/if}</span>
        </div>
      {:else}
        <div class="grow"></div>
      {/if}
      {#if key && taintLabel(run.taint)}
        <!-- What this conversation has touched. With the leak guard lifted
             for personas (D11), this is what is left to say it. -->
        <span class="chip taint" title="what this conversation has touched">{taintLabel(run.taint)}</span>
      {/if}
    {:else}
      <div class="dtitle grow">Personas</div>
      {#if !making}
        <button class="newbtn" onclick={startMaking}>
          <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg>
          New
        </button>
      {/if}
    {/if}
    {#if key && chatModel}
      <ModelChip model={chatModel} />
    {/if}

    <button
      class="lockbtn"
      class:on={!!token}
      title={token ? 'showing locked personas — tap to hide them' : 'show locked personas'}
      aria-label={token ? 'hide locked personas' : 'show locked personas'}
      aria-pressed={!!token}
      onclick={() => (token ? relock() : data?.has_password ? (sheet = true) : unlock())}
    >
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
        {#if token}<path d="M7 11V7a5 5 0 019.9-1M5 11h14v10H5z" />{:else}<path d="M7 11V7a5 5 0 0110 0v4M5 11h14v10H5z" />{/if}
      </svg>
    </button>
  </header>

  {#if error}
    <div class="warnline pad">{error}</div>
  {/if}

  {#if !chosen && making}
    <div class="scroll">
      <div class="dtitle">New persona</div>
      <div class="form">
        <label class="field">Name <span class="hint">lowercase, for the folder and the terminal</span>
          <input class="editbox" bind:value={making.name} placeholder="mara" autocapitalize="off" />
        </label>
        <label class="field">Display name <span class="hint">how they are shown</span>
          <input class="editbox" bind:value={making.display} placeholder="Mara" />
        </label>
        <div class="field">Relationship <span class="hint">templates you can edit; pick any, or none</span>
          <div class="chips">
            {#each authoring?.relationships ?? [] as r (r.name)}
              <button class="chipbtn" class:active={making.relationships.includes(r.name)}
                onclick={() => (making.relationships = toggle(making.relationships, r.name))}>{r.name.replaceAll('_', ' ')}</button>
            {/each}
            <button class="chipbtn newchip" onclick={() => startAdding('relationship')}>+ new</button>
          </div>
          {#if adding?.kind === 'relationship'}
            <div class="adding">
              <input class="editbox" bind:value={adding.name} placeholder="mentor" autocapitalize="off" />
              <textarea class="editbox" rows="6" bind:value={adding.text}
                placeholder={'How someone in this relationship behaves — e.g.\n- asks what you tried first\n- holds you to what you said you would do'}></textarea>
              <div class="btnrow">
                <button class="abtn" onclick={() => (adding = null)}>Cancel</button>
                <button class="abtn primary" disabled={busy || !personaName(adding.name)} onclick={addNew}>Add relationship</button>
              </div>
            </div>
          {/if}
        </div>
        {#if authoring?.characters?.length}
          <label class="field">Portrait <span class="hint">a character from the library</span>
            <select class="editbox" bind:value={making.character}>
              <option value="">none</option>
              {#each authoring.characters as c}<option value={c}>{c}</option>{/each}
            </select>
          </label>
        {/if}
        <div class="field">Groups <span class="hint">personas in a group share an about-me and files</span>
          <div class="chips">
            {#each authoring?.groups ?? [] as g}
              <button class="chipbtn" class:active={making.groups.includes(g)}
                onclick={() => (making.groups = toggle(making.groups, g))}>{g}</button>
            {/each}
            <button class="chipbtn newchip" onclick={() => startAdding('group')}>+ new</button>
          </div>
          {#if adding?.kind === 'group'}
            <div class="adding">
              <input class="editbox" bind:value={adding.name} placeholder="kelp" autocapitalize="off" />
              <input class="editbox" bind:value={adding.text} placeholder="what the group is for (optional)" />
              <div class="btnrow">
                <button class="abtn" onclick={() => (adding = null)}>Cancel</button>
                <button class="abtn primary" disabled={busy || !personaName(adding.name)} onclick={addNew}>Add group</button>
              </div>
            </div>
          {/if}
        </div>
        <label class="lockline"><input type="checkbox" bind:checked={making.locked} /> Locked — hidden until the library is unlocked</label>
        <div class="btnrow">
          <button class="abtn" onclick={() => (making = null)}>Cancel</button>
          <button class="abtn primary" disabled={busy || !personaName(making.name)} onclick={make}>Make</button>
        </div>
        <div class="barnote">Yours, so approved at once. Next you write who they are.</div>
      </div>
    </div>
  {:else if !chosen}
    <div class="scroll">
      <!-- No count of what the lock hides (owner ruling, 2026-09-30, #425):
           a page with every persona locked reads like one with none. -->
      {#if data && personas.length === 0}
        <div class="empty">
          No personas yet.
          <button class="abtn primary" onclick={startMaking}>Make your first persona</button>
        </div>
      {/if}
      <!-- A contacts list, not a wall of tiles: who they are to you reads at
           a glance, and a phone shows a dozen rather than three. -->
      <!-- Proposed in a chat, waiting on the owner (ruled 2026-10-01): above
           the rest, as the library's Waiting pane is its own. -->
      {#if waitingSplit.waiting.length}
        <div class="earlier">Waiting for you</div>
        <div class="plist waitlist">
          {#each waitingSplit.waiting as p (p.name)}
            <button class="prow" onclick={() => choose(p)}>
              {@render avatar(p, 48)}
              <span class="pbody">
                <span class="pname">
                  {p.display}
                  {#if p.locked}<svg class="glyph" viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" role="img" aria-label="hidden behind the library lock"><path d="M7 11V7a5 5 0 0110 0v4M5 11h14v10H5z" /></svg>{/if}
                </span>
                <span class="prel">{relationshipLabel(p) || 'no relationship'}</span>
              </span>
              <span class="badge">proposed</span>
              <svg class="chev" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 6l6 6-6 6" /></svg>
            </button>
          {/each}
        </div>
        {#if waitingSplit.rest.length}<div class="earlier">Personas</div>{/if}
      {/if}
      {#if waitingSplit.rest.length}
        <div class="plist">
          {#each waitingSplit.rest as p (p.name)}
            <button class="prow" onclick={() => choose(p)}>
              {@render avatar(p, 48)}
              <span class="pbody">
                <span class="pname">
                  {p.display}
                  {#if p.locked}<svg class="glyph" viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" role="img" aria-label="hidden behind the library lock"><path d="M7 11V7a5 5 0 0110 0v4M5 11h14v10H5z" /></svg>{/if}
                </span>
                <span class="prel">{relationshipLabel(p) || 'no relationship'}</span>
              </span>
              {#if !p.approved}
                <span class="badge">not approved</span>
              {:else if p.problems.length}
                <span class="badge">{p.problems.length} to fix</span>
              {/if}
              <svg class="chev" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 6l6 6-6 6" /></svg>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  {:else}
    <div class="scroll" bind:this={scroller}>
      {#if editing}
        <div class="chips">
          {#each OWNER_FILES as [file, label]}
            <button class="chipbtn" class:active={editing.file === file} onclick={() => pickFile(file)}>{label}</button>
          {/each}
        </div>
        <div class="modeline">
          {#if current.form?.problem}
            <span class="warnline">The form can't read this file — {current.form.problem}. Fix it as text.</span>
          {:else if asForm}
            <button
              class="linkbtn"
              disabled={formDirty}
              title={formDirty ? 'Save or discard the changes first' : ''}
              onclick={() => setAsText(true)}
            >Edit as text</button>
          {:else if hasForm}
            <button
              class="linkbtn"
              disabled={textDirty}
              title={textDirty ? 'Save or undo the text first' : ''}
              onclick={() => setAsText(false)}
            >Edit as form</button>
          {/if}
        </div>
        {#if editing.file === 'settings'}
          <!-- The lock is a setting of the persona's, not an action of the
               page's (owner, 2026-10-01). It lives in state.toml, written by
               the server, so it is a switch here rather than a form field. -->
          <!-- A button drawn from the state, as the settings form's toggles
               are: a refused lock leaves the persona as it was, and an input
               the click had already flipped would say otherwise (review of
               #491). -->
          <div class="lockrow">
            <span class="locktext">
              Hide behind the library lock
              <span class="hint" id="lockhint">{lockWaitsNow ? `save or undo your changes in ${waitingTabs.map((f) => OWNER_FILES.find(([k]) => k === f)?.[1]).join(' and ')} first — locking closes the editor` : chosen.locked ? 'shown only while the library is unlocked' : token ? 'locking hides it until the library is unlocked' : 'locking hides it now, and closes the editor, until the library is unlocked'}</span>
            </span>
            <!-- The settings form's own switch (form.css), so it looks and
                 focuses as the toggles below it do. -->
            <button type="button" role="switch" class="tf-switch" aria-checked={chosen.locked} aria-label="Hide behind the library lock" aria-describedby="lockhint" disabled={busy || lockWaitsNow} onclick={() => setLocked(!chosen.locked)}><span class="tf-knob"></span></button>
          </div>
        {/if}
        {#if asForm && current.form.form}
          {#key current.digest}
            <TomlForm
              form={current.form.form}
              values={current.form.values}
              bind:draft={current.formDraft}
              {busy}
              onsave={saveForm}
              onclose={() => (editing = null)}
            />
          {/key}
        {:else if asForm}
          {#key `${editing.file}:${current.digest}`}
            <MdForm
              doc={current.form.doc}
              fixed={current.form.fixed ?? []}
              labels={MD_LABELS[editing.file] ?? {}}
              bind:draft={current.formDraft}
              {busy}
              onsave={saveDoc}
              onclose={() => (editing = null)}
            />
          {/key}
        {:else}
          <textarea
            class="editbox filebox"
            spellcheck={editing.file !== 'settings'}
            autocorrect="off"
            autocapitalize="off"
            bind:value={editing.text}
          ></textarea>
        {/if}
        <div class="barnote">
          {#if asForm && editing.file === 'settings'}
            Changes are set in place in <code>persona.toml</code> — your comments in it stay.
          {:else if asForm}
            Saved in one tidy layout. Notes stay in the file as comments and never reach a chat.
          {:else if editing.file === 'settings'}
            A setting that would not load is refused, and the file stays as it was. A line starting with <code>#</code> is a note to yourself.
          {:else}
            Text inside <code>&lt;!-- --&gt;</code> is a note to yourself: kept, never sent to a chat.
          {/if}
        </div>
        {#if editing.saved}<div class="barnote ok">{editing.saved}</div>{/if}
        {#each editing.problems ?? [] as problem}<div class="warnline">{problem}</div>{/each}
        {#if !asForm}
          <div class="btnrow">
            <button class="abtn" onclick={() => (editing = null)}>Close</button>
            <button class="abtn primary" disabled={busy || !textDirty} onclick={saveText}>Save</button>
          </div>
        {/if}
      {:else if !key}
        <!-- A profile: the persona first, one primary action, and the rest
             quieter (design critique: Edit and Lock outranked starting a chat). -->
        <section class="hero">
          {#if chosen.portrait}
            <!-- The picture is its own control (owner, 2026-10-01): tap it to
                 frame it, rather than hunting in a menu. -->
            <button type="button" class="avatarbtn" disabled={busy} aria-label={`Adjust ${chosen.display}'s picture`} title="Adjust the picture" onclick={() => openFraming()}>
              {@render avatar(chosen, 72)}
            </button>
          {:else}
            {@render avatar(chosen, 72)}
          {/if}
          <div class="herotext">
            <div class="heroname">{chosen.display}</div>
            <div class="herochips">
              <!-- Disclosure is the harness's (§12.1): this is an AI, on the
                   page that reads most like a contact. -->
              {#if disclosed}<span class="ai">AI</span>{/if}
              {#if chosen.locked}<svg class="glyph" viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" role="img" aria-label="hidden behind the library lock"><path d="M7 11V7a5 5 0 0110 0v4M5 11h14v10H5z" /></svg>{/if}
              {#each chosen.relationship ?? [] as r}<span class="rchip">{r.replaceAll('_', ' ')}</span>{/each}
              <span class="ver">v{chosen.version}</span>
            </div>
          </div>
          <div class="herotools">
            <button class="iconbtn" aria-label="Edit {chosen.display}" title="Edit" onclick={() => openEditor('identity')}>
              <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 20h4L19 9l-4-4L4 16v4zM13.5 6.5l4 4" /></svg>
            </button>
          </div>
        </section>
        {#if isProposal(chosen)}
          <!-- A proposal: read here, approved as shown. -->
          <section class="reviewcard" aria-label="Proposal waiting for approval">
            <div class="reviewhead">Waiting for your approval</div>
            <div class="reviewnote" class:bad={chosen.origin !== 'model_clean'}>{proposalOrigin(chosen.origin)}</div>
            {#if reviewNote}<div class="warnline">{reviewNote}</div>{/if}
            {#if review}
              <!-- Its relationships are the header's chips; the voice and the
                   tools those relationships grant are not, and approval is
                   of what was seen. -->
              {#if review.voice}<div class="reviewmeta">voice: {review.voice}</div>{/if}
              <div class="reviewmeta">tools: {review.tools?.length ? review.tools.join(', ') : 'none'}</div>
              {#if review.character}
                <div class="reviewchar">
                  {#if review.character.portrait}<img src={review.character.portrait} alt="" />{/if}
                  <div>
                    <div class="reviewcharname">
                      {review.character.name}{#if review.character.waiting}<span class="badge">portrait waiting too</span>{/if}
                    </div>
                    <div class="reviewchartext">{review.character.text}</div>
                  </div>
                </div>
              {/if}
              <div class="reviewlabel">Who they are</div>
              <div class="reviewtext"><ChatProse text={review.identity} /></div>
              {#if review.motivation?.trim()}
                <div class="reviewlabel">What they want</div>
                <div class="reviewtext"><ChatProse text={review.motivation} /></div>
              {/if}
              <div class="reviewbtns">
                <button class="abtn" disabled={busy} onclick={rejectProposal}>{rejectArmed ? 'Tap again to reject' : 'Reject'}</button>
                <button class="abtn primary" disabled={busy} onclick={approveProposal}>
                  {review.character?.waiting ? 'Approve both' : 'Approve'}
                </button>
              </div>
            {:else if !reviewNote}
              <div class="loadingline">reading…</div>
            {/if}
          </section>
        {:else}
          {#if !chosen.approved}
            <div class="warnline">
              {chosen.display} is not approved — <code>mecha persona approve {chosen.name}</code> after reading them.
            </div>
          {/if}
          {#each chosen.problems as problem}
            <div class="warnline">{problem}</div>
          {/each}
        {/if}
        <div class="startbox" class:gone={isProposal(chosen)}>
          {#if showGoal || goal}
            <input
              class="editbox"
              placeholder="What you want from this chat (optional)"
              maxlength="2000"
              bind:value={goal}
            />
          {/if}
          <button class="abtn primary wide" disabled={busy || !chosen.approved} onclick={start}>Start a chat</button>
          {#if !showGoal && !goal}
            <button class="linkbtn quiet goalink" onclick={() => (showGoal = true)}>Set a goal for this chat</button>
          {/if}
        </div>
        <!-- The safety switches are not stated here (owner ruling,
             2026-10-01): they are the persona's settings, read and changed
             in its editor; the usage meters stay. -->
        {#if doseLine(chosen.dose)}
          <div class="status">
            <span class="stat">
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="8.5" /><path d="M12 7.5V12l3 2" /></svg>
              {doseLine(chosen.dose)}
            </span>
          </div>
        {/if}
        <!-- Its files (§10): what every chat with it can read and cite.
             Added here or dropped into its folder; shared ones come from a
             group's folder or everyone's. Small tiles, so the add control
             never reads as a second "Start a chat". -->
        <div class="earlier">Files</div>
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="tiles"
          class:dropping
          ondragover={(e) => { if (busy || !carriesFiles(e.dataTransfer)) return; e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; dropping = true; }}
          ondragleave={() => (dropping = false)}
          ondrop={(e) => { e.preventDefault(); dropping = false; if (!busy) dropSources(e.dataTransfer); }}
        >
          {#each sources as s (s.name)}
            {@const kind = fileKind(s.name)}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div class="tile openable" class:bad={!!s.unreadable} role="button" tabindex="0" title={`${s.name} — ${sourceLine(s)}${s.unreadable ? `\n${s.unreadable}` : ''}`}
              onclick={() => openFile(s)} onkeydown={(e) => e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), openFile(s))}>
              <span class="tkind">{kind}</span>
              <span class="tname">{s.name.replace(/^@[^/]+\//, '')}</span>
              <span class="tstate" class:busy={s.processing}>{sourceState(s)}</span>
              {#if s.shared}
                <!-- A group's or everyone's: removed from its own folder, not here. -->
                <svg class="tshared" viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" role="img" aria-label="shared"><circle cx="9" cy="8" r="3" /><circle cx="17" cy="9" r="2.5" /><path d="M3.5 19c.6-3 2.8-4.5 5.5-4.5s4.9 1.5 5.5 4.5M15 14.6c2.6-.3 4.8 1 5.5 3.9" /></svg>
              {:else}
                <button class="trm" aria-label="Remove {s.name}" title="Remove" disabled={busy} onclick={(e) => { e.stopPropagation(); removeSource(s.name); }}>
                  <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
                </button>
              {/if}
            </div>
          {/each}
          {#each uploading as u, j (j)}
            <div class="tile uploading" title={u}>
              <span class="tkind">{fileKind(u)}</span>
              <span class="tname">{u}</span>
              <span class="tstate busy">uploading…</span>
            </div>
          {/each}
          <label class="tile addtile" class:off={busy} title="Add a paper, notes or a picture for it to read — or drop it here">
            <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg>
            <span class="tname">{sources.length ? 'Add' : 'Add a file'}</span>
            <input type="file" multiple accept=".pdf,.png,.jpg,.jpeg,.webp,.gif,.md,.markdown,.txt" disabled={busy}
              onchange={(e) => { addSources([...e.currentTarget.files]); e.currentTarget.value = ''; }} />
          </label>
        </div>
        {#if sourcesNote}<div class="warnline">{sourcesNote}</div>{/if}
        {#if history.length || historyLoading || historyNote}
          <div class="earlier">Earlier chats</div>
        {/if}
        {#if historyNote}<div class="warnline">{historyNote}</div>{/if}
        {#if !history.length && historyLoading}
          <div class="loadingline">loading…</div>
        {/if}
        {#if history.length}
          <div class="plist">
            {#each history as h (h.id)}
              <button class="hrow" disabled={busy} onclick={() => resume(h.id)}>
                <span class="htitle">{chatHeadline(h) ?? 'A chat'}</span>
                <span class="when">{when(h.created)} · {clock(h.created)}</span>
                <svg class="chev" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 6l6 6-6 6" /></svg>
              </button>
            {/each}
          </div>
        {/if}
      {:else}
        <!-- No banner: the "AI" tag in the header says it (owner ruling,
             2026-09-30; the §12.1 disclosure stays in the harness's voice). -->
        {#if refused.length}
          <div class="barnote">
            Not given here: {refused.map((r) => `${r.tool} (${r.why})`).join('; ')}.
          </div>
        {/if}
        {#each run.entries as entry, i (i)}
          {#if entry.kind === 'user'}
            <div class="bubble" class:queued={entry.queued}>
              {ownWords(entry.text)}{#if entry.queued}<span class="queued-tag">{entry.delivery === 'discarded' ? 'not delivered — send again' : entry.delivery === 'delivered' ? 'steered' : 'queued'}</span>{/if}
            </div>
          {:else if entry.kind === 'assistant'}
            <!-- Each citation as the harness checked it (§10.4): "quoted" is
                 all a check can say — a real quote may support the wrong claim.
                 One that was found opens its page. -->
            <div class="answer"><ChatProse text={entry.text} cites={cites.get(i)} onCite={openCited} actions={chosen.display} listen={isShown(features.rows, 'calls') ? { chat: key, unlock: chosen.locked ? token : null } : null} download /></div>
            {#if !run.running && entry.text?.trim()}
              {#if savedReplies[entry.text]}
                <span class="savednote">saved to files as {savedReplies[entry.text]}</span>
              {:else}
                <button class="linkbtn quiet saveline" disabled={busy || savingReplies.has(entry.text)} onclick={() => saveReply(entry.text)}>{savingReplies.has(entry.text) ? 'Saving…' : 'Save to files'}</button>
              {/if}
            {/if}
          {:else if entry.kind === 'tool'}
            {@const status = toolStatus(run.entries, i)}
            {@const picture = pictureOf(entry)}
            {@const tr = toolRun(run.entries, i, (e) => !pictureOf(e), (e) => e.is_error === true)}
            {#if tr.first}
              <div class="tool" class:err={status === 'failed'}>
                {entry.name}{tr.count > 1 ? ` ×${tr.count}` : ''}{status === 'failed' ? ' — failed' : status === 'retried' ? ' — retried' : ''}
              </div>
            {/if}
            <!-- The picture is the answer, not a detail of the call: drawn
                 once, from this chat's own workspace (`persona_chat::download`). -->
            {#if picture && !repeats.has(i)}
              {#if chosen.locked}
                <span class="genimg"><img src={pictureUrl(picture)} alt="generated" loading="lazy" /></span>
              {:else}
                <a class="genimg" href={pictureUrl(picture)} target="_blank" rel="noopener">
                  <img src={pictureUrl(picture)} alt="generated" loading="lazy" />
                </a>
              {/if}
              <button class="genedit" onclick={() => editImage(picture)}>Edit</button>
              <button class="genedit" onclick={() => savePicture(picture)}>Download</button>
              {#if pictureNote?.path === picture}<span class="genfail">not downloaded: {pictureNote.why}</span>{/if}
            {/if}
          {:else if entry.kind === 'notice'}
            <div class="notice">{entry.text}</div>
          {:else if entry.kind === 'crisis'}
            {#if dismissed.has(entry.id)}
              <button class="linkbtn" onclick={() => { dismissed.delete(entry.id); dismissed = new Set(dismissed); }}>
                support resources
              </button>
            {:else}
              <!-- The plain voice, not the persona (§12.2): the persona paused
                   on this message and did not answer it. -->
              <div class="crisis" role="alert">
                <div class="crisistext">{entry.text}</div>
                <button class="abtn" onclick={() => (dismissed = new Set([...dismissed, entry.id]))}>Close</button>
              </div>
            {/if}
          {/if}
          {#if noPicture.has(i)}
            <!-- The page's word, from the tool rows, not the persona's: a
                 reply can say "here you go" over a call that drew nothing. -->
            <div class="notice">No picture was made in this reply.</div>
          {/if}
        {/each}
        {#if run.streaming}
          <div class="answer"><ChatProse text={run.streaming} /></div>
        {/if}
        {#if waitingLine(run, chosen.display, now)}
          <!-- A slow local model must never look broken (owner, 2026-09-30). -->
          <!-- Announced once, not once a second: the ticking clock is for
               the eye (review of #431). -->
          <div class="waiting">
            <span class="dots" aria-hidden="true"><i></i><i></i><i></i></span>
            <span aria-hidden="true">{waitingLine(run, chosen.display, now)}</span>
            <span class="sr" role="status">{waitingLine(run, chosen.display, 0)?.replace(/ \d+:\d\d$/, '')}</span>
          </div>
        {/if}
        {#if safety?.resources && (crisisShown || run.entries.some((e) => e.kind === 'crisis'))}
          <!-- Offered once this chat has shown a crisis card, and from then
               on one tap away whatever a reload did to the card (owner ruling,
               2026-09-30; review of #418). -->
          {#if showResources}
            <div class="crisis" role="note">
              <div class="crisistext">{safety.resources}</div>
              <button class="abtn" onclick={() => (showResources = false)}>Close</button>
            </div>
          {:else}
            <button class="linkbtn quiet" onclick={() => (showResources = true)}>support resources</button>
          {/if}
        {/if}
      {/if}
    </div>
    {#if key}
      {#if attachments.length}
        <div class="attach-row">
          {#each attachments as p, i}
            <button class="attach-chip" title="remove" onclick={() => attachments.splice(i, 1)}>
              {p.split('/').pop()} ✕
            </button>
          {/each}
        </div>
      {/if}
      <div class="composer">
        <input type="file" multiple hidden bind:this={fileInput} onchange={uploadPicked} />
        <button class="attachbtn" disabled={uploads > 0} onclick={() => fileInput?.click()} aria-label="Attach a file" title="attach a file — it lands in this chat's inbox/">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12.5l-8.2 8.2a5.5 5.5 0 01-7.8-7.8L13.6 4.3a3.7 3.7 0 015.2 5.2l-8.4 8.4a1.85 1.85 0 01-2.6-2.6l7.8-7.8" /></svg>
        </button>
        <div class="cfield">
          <textarea
            rows="1"
            use:grow={input}
            placeholder={`Say something to ${chosen.display}`}
            aria-label={`Message to ${chosen.display}`}
            bind:value={input}
            onkeydown={onKey}
          ></textarea>
          <!-- Send stays during a run: it steers, and a phone has no Enter to
               spare for that. -->
          <button class="send" aria-label={run.running ? 'Steer' : 'Send'} title={run.running ? 'Steer' : 'Send'} disabled={!input.trim() && !attachments.length} onclick={send}>
            <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 19V5M6 11l6-6 6 6" /></svg>
          </button>
        </div>
        {#if isShown(features.rows, 'calls')}
          <!-- A call speaks into this chat, in the persona's voice (§11) —
               beside send, where the assistant's chat keeps its own. -->
          <button class="attachbtn" title={`call ${chosen.display}`} aria-label={`call ${chosen.display}`} onclick={() => caller?.start()}>
            <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="M4 10v4M8 7v10M12 4v16M16 7v10M20 10v4" /></svg>
          </button>
        {/if}
        {#if run.running}
          <button class="stopbtn" aria-label="Stop" title="Stop" onclick={stop}>
            <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true"><rect x="6" y="6" width="12" height="12" rx="2" /></svg>
          </button>
        {/if}
      </div>
    {/if}
  {/if}

  {#if fileSheet && chosen}
    <button class="scrim" aria-label="close" onclick={() => (fileSheet = null)}></button>
    <div class="sheet citedsheet" role="dialog" aria-label="file">
      <div class="sheet-grip"></div>
      <div class="sheet-text">{fileSheet.source.name}</div>
      <div class="barnote">{sourceLine(fileSheet.source)}{fileSheet.source.unreadable ? ` — ${fileSheet.source.unreadable}` : ''}</div>
      {#if fileSheet.text != null}
        <div class="citedtext">{fileSheet.text}</div>
      {:else if fileSheet.note}
        <div class="barnote">{fileSheet.note}</div>
      {/if}
      <div class="sheetacts">
        <!-- The unlock token only where the persona is locked, as a picture's
             link carries it: a download keeps its URL in the browser's
             history (review of #479). -->
        <a class="abtn" href={sourceFileUrl(chosen.name, fileSheet.source.name, 'file', chosen.locked ? token : null)} download>Download</a>
        {#if fileSheet.text == null && fileSheet.source.ready}
          <button class="abtn" disabled={fileSheet.loading} onclick={readFileText}>{fileSheet.loading ? 'Reading…' : 'Show its text'}</button>
        {/if}
        <button class="abtn" onclick={() => (fileSheet = null)}>Close</button>
      </div>
    </div>
  {/if}

  {#if citedPage}
    <button class="scrim" aria-label="close" onclick={() => (citedPage = null)}></button>
    <div class="sheet citedsheet" role="dialog" aria-label="cited page">
      <div class="sheet-grip"></div>
      <div class="sheet-text">{citedPage.file}{citedPage.page != null ? ` · p. ${citedPage.page}${citedPage.through != null && citedPage.through !== citedPage.page ? `–${citedPage.through}` : ''}` : ''}</div>
      {#if citedPage.loading}
        <div class="barnote">Reading…</div>
      {:else if citedPage.error}
        <div class="barnote">{citedPage.error}</div>
      {:else}
        <div class="citedtext">{citedPage.before}<mark>{citedPage.marked}</mark>{citedPage.after}</div>
        <div class="barnote">{citedPage.marked ? 'The page as this chat read it. The marked words are quoted; whether they support what was said is not checked.' : 'The page as this chat read it. The quote could not be placed on it.'}</div>
      {/if}
      <button class="abtn" onclick={() => (citedPage = null)}>Close</button>
    </div>
  {/if}

  {#if framing && chosen?.portrait}
    <button class="scrim" aria-label="close" onclick={() => (framing = null)}></button>
    <!-- It can open unasked (a new portrait), so it says what it is; Escape
         closes it from the window (review of #491). -->
    <div class="sheet framesheet" role="dialog" aria-label={`Adjust ${chosen.display}'s picture`}>
      <div class="sheet-grip"></div>
      <div class="sheet-text">Adjust {chosen.display}'s picture</div>
      <div
        class="framer"
        style="width:{FRAME_SIZE}px;height:{FRAME_SIZE}px"
        role="group"
        tabindex="0"
        aria-label="the picture in its circle: drag, or use the arrow keys, to move it"
        onpointerdown={frameDown}
        onpointermove={frameMove}
        onpointerup={frameUp}
        onpointercancel={frameUp}
        onkeydown={(e) => {
          // An arrow moves the picture as a drag that way would.
          const step = { ArrowLeft: [-8, 0], ArrowRight: [8, 0], ArrowUp: [0, -8], ArrowDown: [0, 8] }[e.key];
          if (step) {
            e.preventDefault();
            framing.frame = dragFrame(framing.frame, step[0], step[1], FRAME_SIZE, framing.aspect);
          }
        }}
      >
        <img
          src={chosen.portrait}
          alt=""
          draggable="false"
          style={frameStyle(framing.frame)}
          onload={(e) => {
            const { naturalWidth: w, naturalHeight: h } = e.currentTarget;
            if (framing && w > 0 && h > 0) framing.aspect = w / h;
          }}
        />
      </div>
      <label class="zoomline">
        <span>zoom</span>
        <input type="range" min="1" max={MAX_FRAME_ZOOM} step="0.05" bind:value={framing.frame.zoom} />
      </label>
      <div class="framebtns">
        <button class="abtn" disabled={busy} onclick={() => saveFrame(null)}>Reset</button>
        <button class="abtn" disabled={busy} onclick={() => (framing = null)}>Cancel</button>
        <button class="abtn primary" disabled={busy} onclick={() => saveFrame(framing.frame)}>Save</button>
      </div>
    </div>
  {/if}

  {#if sheet}
    <button class="scrim" aria-label="close" onclick={() => (sheet = false)}></button>
    <div class="sheet">
      <div class="sheet-grip"></div>
      <div class="sheet-text">Show locked personas</div>
      <input class="editbox" type="password" placeholder="library lock password" bind:value={password}
        onkeydown={(e) => e.key === 'Enter' && unlock()} />
      <button class="abtn primary" disabled={busy || !password} onclick={unlock}>Unlock</button>
      <div class="barnote">The image library's password. Locking hides; it is not encryption.</div>
    </div>
  {/if}

  {#if key && chosen}
    <!-- Idle until the header's call button: it ends with the chat, and the
         token goes only where the persona is locked, as a download's does. -->
    <PersonaCall
      bind:this={caller}
      chatKey={key}
      token={chosen.locked ? token : null}
      display={chosen.display}
      face={callFace}
    />
  {/if}
</div>

{#snippet callFace()}{@render avatar(chosen, 112)}{/snippet}

{#if imageEdit}
  <EditModal
    src={imageEdit.src}
    path={imageEdit.path}
    initial={imageEdit.initial}
    busy={imageEdit.busy}
    error={imageEdit.error}
    onsend={sendEdit}
    onclose={() => (imageEdit = null)}
  />
{/if}

<style>
  .page { flex: 1; display: flex; flex-direction: column; min-height: 0; position: relative; }
  .head { display: flex; align-items: center; gap: 8px; padding: 12px var(--gutter) 0; padding-right: 56px; min-height: 44px; }
  .grow { flex: 1; }
  .who { flex: 1; display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .dtitle { font-family: var(--mono); font-size: 13px; color: var(--accent-400); overflow-wrap: anywhere; }
  /* Muted, not amber: amber is the taint chip's (Chat.svelte's rule), and
     "this is an AI" is not a security posture. */
  .ai { color: var(--text-muted); border: 1px solid var(--accent-700); border-radius: var(--radius-chip); padding: 0 5px; margin-right: 2px; }
  .chip.taint { flex-shrink: 0; font-family: var(--mono); font-size: 10px; color: var(--hazard); border: 1px solid var(--hazard); border-radius: var(--radius-chip); padding: 2px 6px; }
  .crisis { display: flex; flex-direction: column; gap: 10px; background: var(--surface); border: 1px solid var(--accent-500); border-radius: var(--radius); padding: 14px; }
  .crisistext { font-size: 14px; line-height: 1.55; white-space: pre-wrap; color: var(--text); }
  .crisis .abtn { align-self: flex-start; }
  .linkbtn.quiet { color: var(--text-muted); font-size: 11px; }
  .linkbtn { align-self: flex-start; background: none; border: none; padding: 0; color: var(--accent-400); font-size: 12px; text-decoration: underline; cursor: pointer; }
  .lockbtn { flex-shrink: 0; display: flex; align-items: center; justify-content: center; min-height: 40px; min-width: 40px; padding: 0; background: none; border: 1px solid var(--accent-900); border-radius: var(--radius-chip); color: var(--text-muted); cursor: pointer; }
  .lockbtn.on { color: var(--hazard); border-color: var(--hazard); }
  .backbtn { background: none; border: none; color: var(--text-muted); min-width: 44px; min-height: 44px; margin: -10px 0 -10px -12px; cursor: pointer; display: flex; align-items: center; justify-content: center; }
  .scroll { flex: 1; overflow-y: auto; padding: 14px var(--gutter); display: flex; flex-direction: column; gap: 10px; }
  .scroll > * { flex-shrink: 0; }
  .pad { padding: 8px var(--gutter) 0; }
  .badge { align-self: flex-start; margin: 2px 10px 0; font-family: var(--mono); font-size: 10px; color: var(--hazard); border: 1px solid var(--hazard); border-radius: var(--radius-chip); padding: 1px 6px; }
  .when { font-family: var(--mono); font-size: 10px; color: var(--accent-700); margin-left: auto; }
  .earlier { font-family: var(--mono); font-size: 11px; letter-spacing: 0.08em; text-transform: uppercase; color: var(--accent-300); margin-top: 14px; }
  .startbox { display: flex; flex-direction: column; gap: 10px; margin-top: 6px; }
  .form { display: flex; flex-direction: column; gap: 12px; max-width: 520px; }
  .field { display: flex; flex-direction: column; gap: 6px; font-size: 13px; color: var(--text); }
  .hint { font-size: 11px; color: var(--text-muted); }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .chipbtn { min-height: 36px; padding: 0 12px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); color: var(--text-muted); font-family: var(--mono); font-size: 12px; cursor: pointer; }
  .chipbtn.active { color: var(--text); background: var(--accent-900); border-color: var(--accent-700); }
  .newchip { border-style: dashed; color: var(--accent-400); }
  .adding { display: flex; flex-direction: column; gap: 8px; margin-top: 4px; padding: 10px; border: 1px solid var(--accent-900); border-radius: var(--radius); }
  .adding .editbox { font-size: 14px; }
  .glyph { vertical-align: -1px; color: var(--text-muted); }
  .lockline { display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--text-muted); }
  .btnrow { display: flex; gap: 10px; }
  .btnrow .abtn { flex: 1; }
  /* No ligatures: JetBrains Mono draws `-->` as an arrow, and a comment's
   * close must look like what it is — the owner reported it as autocorrect. */
  /* `.editbox.filebox`, not `.filebox`: `.editbox` below has the same
     specificity and came later, so the monospace face and no-ligature
     guard here never applied — text mode drew `->` as an arrow (review of
     #457). */
  .editbox.filebox { min-height: 50vh; font-family: var(--mono); font-size: 13px; line-height: 1.5; resize: vertical; font-variant-ligatures: none; font-feature-settings: 'calt' 0, 'liga' 0; }
  .modeline { display: flex; justify-content: flex-end; min-height: 20px; }
  .linkbtn { background: none; border: none; padding: 4px 0; color: var(--accent-400); font-size: 12px; cursor: pointer; }
  .linkbtn:disabled { color: var(--text-muted); cursor: default; }
  .barnote.ok { color: var(--accent-400); }
  .startbox .editbox { flex: 1; margin-bottom: 0; }
  .editbox { width: 100%; background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; padding: 12px 14px; box-sizing: border-box; }
  /* iOS zooms into a field under 16px on focus and stays zoomed — the text
     editor too, not only the forms (owner, 2026-10-01). */
  @media (hover: none) and (pointer: coarse) {
    .editbox, .editbox.filebox, .adding .editbox { font-size: 16px; }
  }
  .abtn { flex-shrink: 0; min-height: 44px; padding: 0 16px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; white-space: nowrap; }
  .abtn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .abtn:disabled { opacity: 0.5; }
  .bubble { align-self: flex-end; max-width: 82%; background: var(--surface); border-radius: var(--radius); padding: 11px 14px; font-size: 14px; line-height: 1.45; white-space: pre-wrap; }
  .bubble.queued { border: 1px solid var(--accent-700); background: var(--bg); }
  .queued-tag { display: block; margin-top: 4px; font-family: var(--mono); font-size: 9px; color: var(--text-muted); }
  .answer { max-width: 92%; font-size: 14px; line-height: 1.5; white-space: pre-wrap; }
  .citedsheet { max-height: 75%; }
  .sheetacts { display: flex; gap: 8px; flex-wrap: wrap; }
  .sheetacts a.abtn { text-decoration: none; display: inline-flex; align-items: center; justify-content: center; }
  .tiles.dropping { outline: 1px dashed var(--accent-400); outline-offset: 4px; border-radius: 12px; }
  .tile.openable { cursor: pointer; }
  .tile.uploading { opacity: 0.75; }
  .goalink { align-self: center; }
  .saveline { margin-top: -4px; }
  .savednote { margin-top: -4px; font-size: 11px; color: var(--text-muted); }
  .citedtext { overflow-y: auto; white-space: pre-wrap; font-size: 14px; line-height: 1.5; padding: 10px 12px; border: 1px solid var(--accent-900); border-radius: 10px; }
  .citedtext mark { background: var(--accent-700); color: var(--text); border-radius: 3px; }
  .tool { font-family: var(--mono); font-size: 12px; color: var(--text-muted); }
  .tool.err { color: var(--hazard); }
  /* As the assistant's chat draws a picture and its Edit button. */
  .genimg { display: block; max-width: min(100%, 512px); }
  .genimg img { display: block; width: 100%; height: auto; border-radius: 8px; }
  .genfail { font-size: 12px; color: var(--hazard); }
  .genedit {
    align-self: flex-start; margin-top: -4px; padding: 4px 12px;
    font-family: var(--mono); font-size: 12px; color: var(--accent-400);
    background: none; border: 1px solid var(--accent-400); border-radius: 999px; cursor: pointer;
  }
  .notice { font-family: var(--mono); font-size: 11px; color: var(--text-muted); }
  /* As the assistant's chat attaches: chips above the composer, an overlay
     while a file is dragged over the page. */
  .attach-row { display: flex; gap: 6px; flex-wrap: wrap; padding: 8px var(--gutter) 0; }
  .attach-chip { font-family: var(--mono); font-size: 11px; color: var(--text); background: var(--accent-900); border: 1px solid var(--accent-700); border-radius: var(--radius-chip); padding: 6px 10px; cursor: pointer; }
  .attachbtn { flex: none; display: flex; align-items: center; justify-content: center; width: 44px; height: 44px; padding: 0; background: transparent; border: none; border-radius: 22px; color: var(--accent-400); cursor: pointer; }
  .attachbtn:disabled { opacity: 0.4; cursor: default; }
  .drop-overlay {
    position: fixed; inset: 0; z-index: 60; pointer-events: none;
    display: flex; align-items: center; justify-content: center; padding: var(--gutter);
    background: color-mix(in srgb, var(--void) 72%, transparent);
    outline: 2px dashed var(--accent-500); outline-offset: -12px;
  }
  .drop-card {
    display: flex; flex-direction: column; align-items: center; gap: 10px; padding: 20px 24px;
    border-radius: var(--radius); background: var(--accent-900); border: 1px solid var(--accent-700);
    font-family: var(--mono); font-size: 12px; color: var(--text); text-align: center;
  }
  .composer { display: flex; gap: 8px; align-items: flex-end; padding: 10px var(--gutter) 14px; border-top: 1px solid var(--accent-900); }
  .barnote { font-size: 11px; color: var(--text-muted); line-height: 1.5; }
  .empty { color: var(--text-muted); font-size: 14px; padding: 24px 0; text-align: center; line-height: 1.6; }
  .warnline { font-size: 12px; color: var(--hazard); line-height: 1.45; }
  code { font-family: var(--mono); font-size: 11px; }
  .scrim { position: absolute; inset: 0; background: rgba(0, 0, 0, 0.45); z-index: 5; border: none; }
  .sheet { position: absolute; left: 0; right: 0; bottom: 0; background: var(--bg); border-top: 1px solid var(--accent-500); border-radius: 16px 16px 0 0; padding: 14px var(--gutter) 28px; display: flex; flex-direction: column; gap: 12px; z-index: 6; }
  .sheet-grip { width: 36px; height: 4px; border-radius: 2px; background: var(--accent-900); align-self: center; }
  .sheet-text { font-size: 15px; font-weight: 500; }
  .framesheet { align-items: stretch; }
  .framer { position: relative; align-self: center; overflow: hidden; border-radius: 50%; touch-action: none; cursor: grab; background: var(--accent-900); box-shadow: 0 0 0 1px var(--accent-500); }
  .framer:active { cursor: grabbing; }
  .framer img { width: 100%; height: 100%; object-fit: cover; user-select: none; -webkit-user-drag: none; pointer-events: none; }
  .zoomline { display: flex; align-items: center; gap: 10px; font-family: var(--mono); font-size: 11px; color: var(--text-muted); }
  .zoomline input { flex: 1; accent-color: var(--accent-400); }
  .framebtns { display: flex; gap: 8px; }
  .framebtns .abtn { flex: 1; }
  .avatar { flex-shrink: 0; display: inline-flex; align-items: center; justify-content: center; overflow: hidden; border-radius: 50%; background: linear-gradient(145deg, var(--accent-700), var(--accent-900)); color: var(--accent-100); font-family: var(--sans); font-weight: 600; }
  .avatar img { width: 100%; height: 100%; object-fit: cover; }
  .pname { display: flex; align-items: center; gap: 6px; font-family: var(--sans); font-size: 15px; font-weight: 600; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .newbtn { display: inline-flex; align-items: center; gap: 6px; min-height: 40px; padding: 0 14px; background: var(--accent-400); border: none; border-radius: 20px; color: var(--void); font-size: 14px; font-weight: 500; cursor: pointer; }
  .plist { display: flex; flex-direction: column; background: var(--bg); border: 1px solid var(--accent-900); border-radius: 14px; overflow: hidden; }
  .prow, .hrow { display: flex; align-items: center; gap: 14px; min-height: 72px; padding: 10px 14px 10px 16px; background: transparent; border: none; text-align: left; color: var(--text); cursor: pointer; }
  .prow + .prow, .hrow + .hrow { border-top: 1px solid var(--accent-900); }
  .prow:hover, .hrow:hover:not(:disabled) { background: var(--surface); }
  .pbody { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 3px; }
  .prel { font-size: 13px; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .prow .badge { margin: 0; align-self: center; }
  .chev { flex-shrink: 0; color: var(--accent-500); }
  .hero { display: flex; align-items: center; gap: 16px; padding: 8px 0 4px; }
  .herotext { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
  .heroname { font-size: 24px; font-weight: 650; letter-spacing: -0.01em; color: var(--text); }
  .herochips { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .rchip { padding: 3px 10px; border-radius: 12px; background: var(--accent-900); color: var(--accent-100); font-size: 12px; }
  .ver { font-family: var(--mono); font-size: 11px; color: var(--text-muted); }
  .herotools { display: flex; align-self: flex-start; gap: 2px; }
  .iconbtn { display: flex; align-items: center; justify-content: center; width: 44px; height: 44px; padding: 0; background: transparent; border: none; border-radius: 10px; color: var(--text-muted); cursor: pointer; }
  .iconbtn:hover { background: var(--surface); color: var(--text); }
  /* The picture as its own control: a ring on hover and focus says it can be
     tapped, and nothing else changes how it looks. */
  .avatarbtn { flex-shrink: 0; padding: 0; background: none; border: none; border-radius: 50%; cursor: pointer; line-height: 0; }
  .avatarbtn:hover :global(.avatar), .avatarbtn:focus-visible :global(.avatar) { box-shadow: 0 0 0 2px var(--accent-400); }
  .lockrow { display: flex; align-items: center; gap: 12px; padding: 12px 14px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: 10px; font-size: 14px; }
  .locktext { flex: 1; min-width: 0; }
  .lockrow .hint { display: block; margin-top: 2px; }
  .lockrow :global(.tf-switch:disabled) { opacity: 0.5; cursor: default; }
  .abtn.wide { width: 100%; }
  .status { display: flex; flex-wrap: wrap; gap: 6px 16px; }
  .stat { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; color: var(--text-muted); }
  .stat svg { color: var(--accent-500); flex-shrink: 0; }
  .htitle { flex: 1; min-width: 0; font-size: 14px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hrow { min-height: 56px; }
  /* A file is a small tile, not a full-width row: the add control is the
     same size as a file, so it never reads as a second primary button, and
     a tile has the square a thumbnail will take. */
  .tiles { display: grid; grid-template-columns: repeat(auto-fill, minmax(78px, 1fr)); gap: 8px; }
  .tile { position: relative; display: flex; flex-direction: column; justify-content: flex-end; gap: 2px; min-width: 0; height: 74px; padding: 8px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: 10px; }
  .tile.bad { border-color: color-mix(in srgb, var(--hazard) 45%, var(--accent-900)); }
  .tkind { position: absolute; top: 8px; left: 8px; padding: 1px 5px; border-radius: 4px; background: var(--accent-900); color: var(--accent-100); font-family: var(--mono); font-size: 9px; letter-spacing: 0.06em; }
  .tname { font-size: 12px; line-height: 1.25; color: var(--text); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tstate { font-family: var(--mono); font-size: 10px; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tstate.busy { color: var(--accent-400); }
  .tile.bad .tstate { color: var(--hazard); }
  .trm { position: absolute; top: 2px; right: 2px; display: flex; align-items: center; justify-content: center; width: 32px; height: 32px; padding: 0; background: transparent; border: none; border-radius: 8px; color: var(--text-muted); cursor: pointer; }
  .tshared { position: absolute; top: 10px; right: 9px; color: var(--accent-500); }
  .trm:hover:not(:disabled) { background: var(--surface); color: var(--text); }
  .addtile { align-items: center; justify-content: center; gap: 4px; border-style: dashed; color: var(--accent-400); cursor: pointer; }
  .addtile .tname { color: inherit; }
  .addtile:hover { background: var(--surface); }
  .addtile.off { opacity: 0.55; cursor: default; }
  .addtile input { position: absolute; width: 1px; height: 1px; opacity: 0; pointer-events: none; }
  .reviewcard { display: flex; flex-direction: column; gap: 10px; padding: 14px; background: var(--bg); border: 1px solid var(--accent-500); border-radius: 14px; }
  .reviewhead { font-size: 15px; font-weight: 600; color: var(--text); }
  .reviewnote { font-size: 12px; color: var(--text-muted); line-height: 1.45; }
  .reviewnote.bad { color: var(--hazard); }
  .reviewmeta { font-family: var(--mono); font-size: 11px; color: var(--text-muted); }
  .reviewchar { display: flex; gap: 12px; align-items: flex-start; }
  .reviewchar img { width: 64px; height: 64px; object-fit: cover; object-position: 50% 20%; border-radius: 10px; flex-shrink: 0; }
  .reviewcharname { display: flex; align-items: center; gap: 8px; font-size: 14px; color: var(--text); }
  .reviewchartext { font-size: 12.5px; color: var(--text-muted); line-height: 1.45; }
  .reviewlabel { font-family: var(--mono); font-size: 11px; letter-spacing: 0.08em; text-transform: uppercase; color: var(--accent-300); margin-top: 4px; }
  .reviewtext { font-size: 14px; line-height: 1.55; color: var(--text); }
  .reviewbtns { display: flex; gap: 8px; margin-top: 4px; }
  .reviewbtns .abtn { flex: 1; }
  .startbox.gone { display: none; }
  .loadingline { font-family: var(--mono); font-size: 11px; color: var(--text-muted); padding: 4px 2px; }
  .hrow .when { margin: 0; font-family: var(--mono); font-size: 11px; color: var(--text-muted); }
  .composer .cfield { flex: 1; display: flex; align-items: flex-end; gap: 6px; padding: 6px 6px 6px 14px; background: var(--surface); border: 1px solid var(--accent-700); border-radius: 22px; }
  .composer .cfield:focus-within { border-color: var(--accent-500); }
  .composer textarea { flex: 1; width: 100%; min-width: 0; height: 36px; max-height: 160px; padding: 8px 0; box-sizing: border-box; resize: none; background: transparent; border: none; color: var(--text); font-family: var(--sans); font-size: 15px; line-height: 1.35; text-align: left; outline: none; font-variant-ligatures: none; }
  .send, .stopbtn { flex-shrink: 0; display: flex; align-items: center; justify-content: center; width: 36px; height: 36px; padding: 0; border: none; border-radius: 50%; cursor: pointer; }
  .send { background: var(--accent-400); color: var(--void); }
  .send:disabled { background: var(--accent-900); color: var(--text-muted); cursor: default; }
  .stopbtn { width: 44px; height: 44px; background: var(--surface); border: 1px solid var(--accent-900); color: var(--text); }
  .waiting { display: flex; align-items: center; gap: 10px; font-size: 13px; color: var(--text-muted); padding: 2px 0; }
  .dots { display: inline-flex; gap: 4px; }
  .dots i { width: 6px; height: 6px; border-radius: 50%; background: var(--accent-400); animation: blink 1.2s infinite ease-in-out; }
  .dots i:nth-child(2) { animation-delay: 0.2s; }
  .dots i:nth-child(3) { animation-delay: 0.4s; }
  @keyframes blink { 0%, 80%, 100% { opacity: 0.25; transform: translateY(0); } 40% { opacity: 1; transform: translateY(-2px); } }
  @media (prefers-reduced-motion: reduce) { .dots i { animation: none; opacity: 0.7; } }
  .pname .ai { font-family: var(--mono); font-size: 10px; font-weight: 400; line-height: 1.4; margin-left: 6px; }
  .herochips .ai { font-family: var(--mono); font-size: 10px; line-height: 1.4; }
  .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
</style>
