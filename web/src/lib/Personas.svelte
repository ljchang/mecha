<script>
  import { tick, untrack } from 'svelte';
  import { apiFetch as fetch } from './api.js';
  import TomlForm from './TomlForm.svelte';
  import MdForm from './MdForm.svelte';
  import ModelChip from './ModelChip.svelte';
  import { repairComments, changesOf } from './tomlform.js';
  import { isDirty as mdDirty } from './mdform.js';
  import {
    listUrl, personaUrl, chatUrl, relationshipLabel, emptyRun, applyEvent, settle,
    taintLabel, safetyLine, doseLine, authoringUrl, personaName, keptCharacter, OWNER_FILES, keptEdits,
    toolStatus, waitingLine,
  } from './persona.js';
  // The Personas tab (PERSONA-DESIGN.md §8; the owner's ruling of
  // 2026-09-29: a tab of its own, not a mode of the assistant's chat).
  //
  // Everything here goes through the persona door (`persona_chat.rs`) —
  // `persona.js` refuses to build any other URL — so nothing on this page can
  // read or write one of the assistant's conversations.
  //
  // The lock is the image library's: the same password, the same token, held
  // only in memory (the no-storage test holds the whole bundle to that).

  let { initial = null } = $props();

  let data = $state(null);
  let error = $state('');
  let token = $state(null);
  let password = $state('');
  let sheet = $state(false);
  let busy = $state(false);

  // The persona being looked at, its earlier chats, and the open chat.
  let chosen = $state(null);
  let history = $state([]);
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
  // A clock for the waiting line, ticking only while a run is live.
  let now = $state(Date.now());
  $effect(() => {
    if (!run.running) return;
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
  let menuOpen = $state(false);
  let goal = $state('');
  let input = $state('');
  let source = null;
  let usedInitial = false;
  let scroller = $state(null);

  const personas = $derived(data?.personas ?? []);

  async function load() {
    error = '';
    try {
      const res = await fetch(listUrl(token));
      if (!res.ok) throw new Error((await res.text()).trim());
      data = await res.json();
      // A token the server no longer honours has lapsed.
      if (token && !data.unlocked) token = null;
      if (chosen) chosen = personas.find((p) => p.name === chosen.name) ?? null;
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
        if (chosen) await loadHistory();
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
      token = (await res.json()).token;
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

  async function relock() {
    const t = token;
    token = null;
    // Off the screen now, before the grid's round trip, not after it.
    if (making && authoring) authoring = { ...authoring, characters: [] };
    // A locked persona's chat closes with the lock: the lock hides (§8.3).
    if (chosen?.locked) toList();
    await load();
    await rereadAuthoring();
    if (t) {
      fetch('/api/library/relock', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ token: t }),
      }).catch(() => {});
    }
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

  // An earlier chat's goal, when it was opened with one: what tells two
  // chats apart. (A session title here is always the automatic
  // "persona: …", so it is never shown — review of #431.)
  const chatGoal = (h) => (h.goal ?? '').trim();

  // One step back at a time: out of a chat or the editor to the persona,
  // and from the persona to the list. The chat's "Done" button was this.
  // The lock never steps: `toList` hides everything at once (review of #431).
  function back() {
    if (key) {
      close();
      key = null;
      run = emptyRun();
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
    menuOpen = false;
    close();
    making = null;
    editing = null;
    chosen = null;
    key = null;
    run = emptyRun();
    history = [];
    refused = [];
  }

  async function choose(p) {
    close();
    chosen = p;
    key = null;
    run = emptyRun();
    refused = [];
    goal = '';
    await loadHistory();
  }

  async function loadHistory() {
    if (!chosen) return;
    try {
      const res = await fetch(personaUrl(chosen.name, '/chats', token));
      history = res.ok ? (await res.json()).chats : [];
    } catch {
      history = [];
    }
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
    run = { ...emptyRun(settle(t.entries, run), t.taint ?? null), running: !!t.running };
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
      await load();
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

  async function setLocked(locked) {
    busy = true;
    try {
      const res = await fetch(personaUrl(chosen.name, '/lock', null), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ locked, unlock: token ?? undefined }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      await load();
      // Locked with no unlock in hand, it is hidden now: back to the list.
      if (!chosen) toList();
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
    const text = input.trim();
    if (!text || !key) return;
    input = '';
    try {
      const res = await fetch(chatUrl(key, '/send'), {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ text, unlock: token ?? undefined }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
    } catch (e) {
      input = text;
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
    {#if p.portrait}<img src={p.portrait} alt="" />{:else}{p.display.slice(0, 1).toUpperCase()}{/if}
  </span>
{/snippet}

<svelte:window onclick={(e) => menuOpen && !e.target.closest?.('.menuwrap') && (menuOpen = false)} />

<div class="page">
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
          <span class="pname">{chosen.display} <span class="ai">AI</span></span>
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
      {#if personas.length}
        <div class="plist">
          {#each personas as p (p.name)}
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
          {@render avatar(chosen, 72)}
          <div class="herotext">
            <div class="heroname">{chosen.display}</div>
            <div class="herochips">
              <!-- Disclosure is the harness's (§12.1): this is an AI, on the
                   page that reads most like a contact. -->
              <span class="ai">AI</span>
              {#if chosen.locked}<svg class="glyph" viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" role="img" aria-label="hidden behind the library lock"><path d="M7 11V7a5 5 0 0110 0v4M5 11h14v10H5z" /></svg>{/if}
              {#each chosen.relationship ?? [] as r}<span class="rchip">{r.replaceAll('_', ' ')}</span>{/each}
              <span class="ver">v{chosen.version}</span>
            </div>
          </div>
          <div class="herotools">
            <button class="iconbtn" aria-label="Edit {chosen.display}" title="Edit" onclick={() => openEditor('identity')}>
              <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 20h4L19 9l-4-4L4 16v4zM13.5 6.5l4 4" /></svg>
            </button>
            <div class="menuwrap">
              <button class="iconbtn" aria-label="More" aria-haspopup="menu" aria-expanded={menuOpen} onclick={() => (menuOpen = !menuOpen)}>
                <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="1.6" /><circle cx="12" cy="12" r="1.6" /><circle cx="19" cy="12" r="1.6" /></svg>
              </button>
              {#if menuOpen}
                <div class="menu" role="menu">
                  <button role="menuitem" class="mitem" disabled={busy} onclick={() => { menuOpen = false; setLocked(!chosen.locked); }}>
                    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M7 11V7a5 5 0 0110 0v4M5 11h14v10H5z" /></svg>
                    {chosen.locked ? 'Stop hiding behind the library lock' : 'Hide behind the library lock'}
                  </button>
                </div>
              {/if}
            </div>
          </div>
        </section>
        {#if !chosen.approved}
          <div class="warnline">
            {chosen.display} is not approved — <code>mecha persona approve {chosen.name}</code> after reading them.
          </div>
        {/if}
        {#each chosen.problems as problem}
          <div class="warnline">{problem}</div>
        {/each}
        <div class="startbox">
          <input
            class="editbox"
            placeholder="A goal for this chat (optional)"
            maxlength="2000"
            bind:value={goal}
          />
          <button class="abtn primary wide" disabled={busy || !chosen.approved} onclick={start}>Start a chat</button>
        </div>
        <div class="status">
          <span class="stat">
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3l7 3v6c0 4.5-3 7.5-7 9-4-1.5-7-4.5-7-9V6l7-3z" /></svg>
            {safetyLine(chosen.safety)}
          </span>
          {#if doseLine(chosen.dose)}
            <span class="stat">
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="8.5" /><path d="M12 7.5V12l3 2" /></svg>
              {doseLine(chosen.dose)}
            </span>
          {/if}
        </div>
        {#if history.length}
          <div class="earlier">Earlier chats</div>
          <div class="plist">
            {#each history as h (h.id)}
              <button class="hrow" disabled={busy} onclick={() => resume(h.id)}>
                {#if chatGoal(h)}
                  <span class="htitle">{chatGoal(h)}</span>
                  <span class="when">{when(h.created)} · {clock(h.created)}</span>
                {:else}
                  <span class="htitle">{when(h.created)}</span>
                  <span class="when">{clock(h.created)}</span>
                {/if}
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
              {entry.text}{#if entry.queued}<span class="queued-tag">{entry.delivery === 'discarded' ? 'not delivered — send again' : entry.delivery === 'delivered' ? 'steered' : 'queued'}</span>{/if}
            </div>
          {:else if entry.kind === 'assistant'}
            <div class="answer">{entry.text}</div>
          {:else if entry.kind === 'tool'}
            {@const status = toolStatus(run.entries, i)}
            <div class="tool" class:err={status === 'failed'}>
              {entry.name}{status === 'failed' ? ' — failed' : status === 'retried' ? ' — retried' : ''}
            </div>
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
        {/each}
        {#if run.streaming}
          <div class="answer">{run.streaming}</div>
        {/if}
        {#if waitingLine(run, chosen.display, now)}
          <!-- A slow local model must never look broken (owner, 2026-09-30). -->
          <div class="waiting" role="status" aria-live="polite">
            <span class="dots" aria-hidden="true"><i></i><i></i><i></i></span>
            {waitingLine(run, chosen.display, now)}
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
      <div class="composer">
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
          <button class="send" aria-label={run.running ? 'Steer' : 'Send'} title={run.running ? 'Steer' : 'Send'} disabled={!input.trim()} onclick={send}>
            <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 19V5M6 11l6-6 6 6" /></svg>
          </button>
        </div>
        {#if run.running}
          <button class="stopbtn" aria-label="Stop" title="Stop" onclick={stop}>
            <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor" aria-hidden="true"><rect x="6" y="6" width="12" height="12" rx="2" /></svg>
          </button>
        {/if}
      </div>
    {/if}
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
</div>

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
  .filebox { min-height: 50vh; font-family: var(--mono); font-size: 13px; line-height: 1.5; resize: vertical; font-variant-ligatures: none; font-feature-settings: 'calt' 0, 'liga' 0; }
  .modeline { display: flex; justify-content: flex-end; min-height: 20px; }
  .linkbtn { background: none; border: none; padding: 4px 0; color: var(--accent-400); font-size: 12px; cursor: pointer; }
  .linkbtn:disabled { color: var(--text-muted); cursor: default; }
  .barnote.ok { color: var(--accent-400); }
  .startbox .editbox { flex: 1; margin-bottom: 0; }
  .editbox { width: 100%; background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; padding: 12px 14px; box-sizing: border-box; }
  .abtn { flex-shrink: 0; min-height: 44px; padding: 0 16px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; white-space: nowrap; }
  .abtn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .abtn:disabled { opacity: 0.5; }
  .bubble { align-self: flex-end; max-width: 82%; background: var(--surface); border-radius: var(--radius); padding: 11px 14px; font-size: 14px; line-height: 1.45; white-space: pre-wrap; }
  .bubble.queued { border: 1px solid var(--accent-700); background: var(--bg); }
  .queued-tag { display: block; margin-top: 4px; font-family: var(--mono); font-size: 9px; color: var(--text-muted); }
  .answer { max-width: 92%; font-size: 14px; line-height: 1.5; white-space: pre-wrap; }
  .tool { font-family: var(--mono); font-size: 12px; color: var(--text-muted); }
  .tool.err { color: var(--hazard); }
  .notice { font-family: var(--mono); font-size: 11px; color: var(--text-muted); }
  .composer { display: flex; gap: 8px; align-items: flex-end; padding: 10px var(--gutter) 14px; border-top: 1px solid var(--accent-900); }
  .barnote { font-size: 11px; color: var(--text-muted); line-height: 1.5; }
  .empty { color: var(--text-muted); font-size: 14px; padding: 24px 0; text-align: center; line-height: 1.6; }
  .warnline { font-size: 12px; color: var(--hazard); line-height: 1.45; }
  code { font-family: var(--mono); font-size: 11px; }
  .scrim { position: absolute; inset: 0; background: rgba(0, 0, 0, 0.45); z-index: 5; border: none; }
  .sheet { position: absolute; left: 0; right: 0; bottom: 0; background: var(--bg); border-top: 1px solid var(--accent-500); border-radius: 16px 16px 0 0; padding: 14px var(--gutter) 28px; display: flex; flex-direction: column; gap: 12px; z-index: 6; }
  .sheet-grip { width: 36px; height: 4px; border-radius: 2px; background: var(--accent-900); align-self: center; }
  .sheet-text { font-size: 15px; font-weight: 500; }
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
  .prow .badge { margin: 0; }
  .chev { flex-shrink: 0; color: var(--accent-500); }
  .hero { display: flex; align-items: center; gap: 16px; padding: 8px 0 4px; }
  .herotext { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 8px; }
  .heroname { font-size: 24px; font-weight: 650; letter-spacing: -0.01em; color: var(--text); }
  .herochips { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
  .rchip { padding: 3px 10px; border-radius: 12px; background: var(--accent-900); color: var(--accent-100); font-size: 12px; }
  .ver { font-family: var(--mono); font-size: 11px; color: var(--text-muted); }
  .herotools { display: flex; align-self: flex-start; gap: 2px; }
  .iconbtn { display: flex; align-items: center; justify-content: center; width: 44px; height: 44px; padding: 0; background: transparent; border: none; border-radius: 10px; color: var(--text-muted); cursor: pointer; }
  .iconbtn:hover, .iconbtn[aria-expanded='true'] { background: var(--surface); color: var(--text); }
  .menuwrap { position: relative; }
  .menu { position: absolute; top: 46px; right: 0; z-index: 4; min-width: 250px; padding: 6px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: 12px; box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45); }
  .mitem { display: flex; align-items: center; gap: 10px; width: 100%; min-height: 44px; padding: 0 12px; background: transparent; border: none; border-radius: 8px; color: var(--text); font-size: 14px; text-align: left; cursor: pointer; }
  .mitem svg { color: var(--text-muted); flex-shrink: 0; }
  .mitem:hover { background: var(--accent-900); }
  .abtn.wide { width: 100%; }
  .status { display: flex; flex-wrap: wrap; gap: 6px 16px; }
  .stat { display: inline-flex; align-items: center; gap: 6px; font-size: 12px; color: var(--text-muted); }
  .stat svg { color: var(--accent-500); flex-shrink: 0; }
  .htitle { flex: 1; min-width: 0; font-size: 14px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hrow { min-height: 56px; }
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
  .pname .ai { font-family: var(--mono); font-size: 10px; font-weight: 400; line-height: 1.4; }
</style>
