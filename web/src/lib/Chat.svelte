<script>
  // The chat view: a rendering of the conversation the server owns, plus a
  // live SSE feed of the run in flight. Sending during a run steers it —
  // the server folds the text into the tool-results turn.
  //
  // Voice rides the voice arc's module (scripts/voice/voice-core.js) —
  // imported by relative path; the module stays framework-free for whatever
  // embeds voice next. Since D3 a call speaks into *this* session: the key
  // travels in the WebRTC offer, the facade resolves it against the same
  // conversation this view is rendering, and spoken turns arrive here over
  // the ordinary SSE feed like any other.
  import { createVoiceSession } from '../../../scripts/voice/voice-core.js';

  let key = $state('main');
  let mode = $state('read_only');
  let rail = $state([]);
  let entries = $state([]);
  let streaming = $state('');
  let running = $state(false);
  let taint = $state(null);
  let usage = $state(null);
  let model = $state('');
  let draft = $state('');
  let error = $state(null);
  let transcriptEl = $state(null);

  // Interim voice-out: the browser's own synthesis reads replies aloud when
  // toggled. Deliberately a stopgap — the real voice mode (Pipecat, the
  // chosen launch voice, barge-in) replaces this when the speech servers
  // land; until then it is the fail-to-a-lesser-mode shape, and marked so.

  function pushEntry(entry) {
    flushStreaming();
    entries.push(entry);
    scrollDown();
  }

  function flushStreaming() {
    if (streaming.trim()) {
      entries.push({ kind: 'assistant', text: streaming });
    }
    streaming = '';
  }

  function scrollDown() {
    queueMicrotask(() => {
      transcriptEl?.scrollTo({ top: transcriptEl.scrollHeight });
    });
  }

  async function load() {
    try {
      const res = await fetch(`/api/chat/${key}`);
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${(await res.text()).trim()}`);
      const data = await res.json();
      entries = data.entries.map((e) =>
        e.kind === 'tool' ? { ...e, pending: false } : e
      );
      running = data.running;
      taint = data.taint;
      model = data.model;
      mode = data.mode ?? 'read_only';
      for (const q of data.questions ?? []) {
        if (!entries.some((e) => e.kind === 'question' && e.qid === q.qid)) {
          entries.push({
            kind: 'question',
            qid: q.qid,
            qkind: q.kind,
            tool: q.tool,
            args: q.args,
            draft: q.draft,
            expanded: false,
            question: q.question,
            options: q.options ?? [],
            freeText: '',
            denying: false,
            denyReason: '',
          });
        }
      }
      if (data.usage?.prompt_tokens) {
        usage = { prompt: data.usage.prompt_tokens, window: data.usage.context_window };
      }
      error = null;
      scrollDown();
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  function subscribe() {
    // tailscale serve injects the identity header on this request too —
    // EventSource cannot set headers, and never needs to here.
    const source = new EventSource(`/api/chat/${key}/events`);
    source.onmessage = (raw) => {
      const ev = JSON.parse(raw.data);
      switch (ev.type) {
        case 'delta':
          streaming += ev.text;
          scrollDown();
          break;
        case 'queued':
          pushEntry({ kind: 'user', text: ev.text, queued: true });
          break;
        case 'user':
          // Words this page did not type — spoken into the same
          // conversation (D3). It is also the only signal that a run
          // started, since nothing local set `running` for it.
          pushEntry({ kind: 'user', text: ev.text, spoken: true });
          running = true;
          break;
        case 'tool':
          pushEntry({ kind: 'tool', name: ev.name, pending: true });
          break;
        case 'tool_result': {
          const open = entries.findLast((e) => e.kind === 'tool' && e.pending);
          if (open) {
            open.pending = false;
            open.is_error = ev.is_error;
          }
          break;
        }
        case 'denied':
          pushEntry({ kind: 'tool', name: ev.name, blocked: true, pending: false });
          break;
        case 'usage':
          usage = { prompt: ev.prompt_tokens, window: ev.context_window };
          break;
        case 'notice':
          pushEntry({ kind: 'notice', text: ev.text });
          break;
        case 'mode':
          // The server is the owner of this, not the tap that asked for it:
          // a change made on the phone has to reach the laptop watching the
          // same session, and a POST whose response was lost must not leave
          // the chip describing a run that is no longer gated that way.
          mode = ev.mode;
          break;
        case 'question':
          pushEntry({
            kind: 'question',
            qid: ev.qid,
            qkind: ev.kind,
            tool: ev.tool,
            args: ev.args,
            draft: ev.draft,
            expanded: false,
            question: ev.question,
            options: ev.options ?? [],
            freeText: '',
            denying: false,
            denyReason: '',
          });
          break;
        case 'question_done':
          entries = entries.filter((e) => !(e.kind === 'question' && e.qid === ev.qid));
          break;
        case 'staged':
          // The reply that produced the draft lands first, then the offer —
          // a card above the sentence explaining it reads as a non sequitur.
          flushStreaming();
          for (const id of ev.ids) offerDraft(id);
          break;
        case 'done':
          flushStreaming();
          running = false;
          taint = { private: ev.taint_private, untrusted: ev.taint_untrusted };
          if (!ev.ok && ev.error) pushEntry({ kind: 'notice', text: ev.error });
          entries = entries.map((e) =>
            e.kind === 'tool' && e.pending ? { ...e, pending: false } : e
          );
          break;
      }
    };
    return source;
  }

  // A draft this run staged, put in front of you rather than left to a badge
  // — `review now`, which the TUI and Slack have always had and this surface
  // never did.
  //
  // The card is built from `/api/outbox/{id}`, never from the event: that
  // endpoint returns the whole reviewable object — every argument, the taint
  // snapshot, and the thread a reply answers — and a reviewer reading one
  // thing while approving another is the failure the outbox exists to
  // prevent. Ids on the wire, bytes from the store.
  async function offerDraft(id) {
    try {
      const res = await fetch(`/api/outbox/${id}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      pushEntry({ kind: 'draft', id, draft: await res.json(), busy: false, showSource: false });
    } catch (e) {
      // "Could not read it back" and "nothing was staged" are opposite
      // findings, so the failure says a draft exists and where it is rather
      // than quietly rendering nothing.
      pushEntry({
        kind: 'notice',
        text: `a draft was staged but could not be read back (${e?.message ?? e}) — it is waiting in your outbox`,
      });
    }
  }

  async function releaseDraft(entry) {
    entry.busy = true;
    try {
      const res = await fetch(`/api/outbox/${entry.id}/approve`, { method: 'POST' });
      if (!res.ok) throw new Error((await res.text()).trim());
      // The card is replaced rather than ticked: it was a question, and a
      // question that has been answered is a fact about what happened.
      entries = entries.map((e) =>
        e === entry ? { kind: 'notice', text: `sent — ${entry.draft.headline || entry.draft.label}` } : e
      );
    } catch (e) {
      entry.busy = false;
      entry.error = String(e?.message ?? e);
    }
  }

  function keepDraft(entry) {
    entries = entries.map((e) =>
      e === entry
        ? { kind: 'notice', text: `left in your outbox — ${entry.draft.headline || entry.draft.label}` }
        : e
    );
  }

  async function loadRail() {
    try {
      const res = await fetch('/api/sessions');
      if (res.ok) rail = (await res.json()).sessions;
    } catch {
      // the rail is a convenience; the transcript is the truth
    }
  }

  function switchTo(k) {
    if (k === key) return;
    key = k;
    entries = [];
    streaming = '';
    usage = null;
    taint = null;
  }

  // The drawer: every conversation this process holds, and the recorded
  // ones from earlier — the pattern every multi-session app converges on,
  // a left panel that expands and collapses. Voice sessions are here too:
  // a brainstorm spoken on a walk resumes as a text chat, same
  // conversation, same taint.
  let drawer = $state(false);
  let history = $state(null);

  async function loadHistory() {
    try {
      const res = await fetch('/api/history');
      if (res.ok) history = (await res.json()).sessions;
    } catch {
      // the drawer is a convenience; the transcript is the truth
    }
  }

  function openDrawer() {
    drawer = true;
    loadRail();
    loadHistory();
  }

  async function resumeSession(id) {
    try {
      const res = await fetch('/api/resume', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ id }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      drawer = false;
      switchTo(data.key);
    } catch (e) {
      pushEntry({ kind: 'notice', text: `resume failed: ${e?.message ?? e}` });
    }
  }

  const sessionLabel = (s) =>
    (s.title ?? s.key).replace(/^(web|voice): /, '') || s.key;

  function newSession() {
    const name = prompt('Session name (lowercase, dashes):');
    if (!name) return;
    const k = name.trim().toLowerCase().replace(/[^a-z0-9_-]/g, '-').slice(0, 32);
    if (k) switchTo(k);
  }

  // Re-subscribe whenever the key changes; the server owns every
  // conversation, so switching is just pointing the rendering elsewhere.
  $effect(() => {
    const source = subscribe();
    load();
    loadRail();
    return () => source.close();
  });
  const railTimer = setInterval(loadRail, 20_000);
  $effect(() => () => clearInterval(railTimer));

  // ---- voice call (overlay over this view) ----
  let voiceOpen = $state(false);
  let vState = $state({ name: 'idle', label: 'connecting' });
  let vEntries = $state([]);
  let vLinked = $state(false);
  let vLevel = $state(0);
  let vSession = null;
  let voicePane = $state(null);

  function vScroll() {
    queueMicrotask(() => voicePane?.scrollTo({ top: voicePane.scrollHeight }));
  }

  function onTranscript({ who, text, interim }) {
    const last = vEntries.at(-1);
    if (last && last.who === who && last.interim) {
      last.text = text;
      last.interim = interim;
    } else {
      vEntries.push({ who, text, interim });
    }
    vScroll();
  }

  // Voice picker and rate, over voice-core's `voiceConfig` contract. Three
  // rules carried from the standalone dock, because two shells over one
  // module must not disagree about how speech sounds:
  //
  //  1. **Render the server's answer, never an optimistic local value.** A
  //     slider reading 1.6x while the worker refused it would be a control
  //     describing a voice you are not hearing.
  //  2. **Preferences are restored once per connection, never re-asserted.**
  //     A settings echo that re-sends on every reply is a loop, and the
  //     thing it loops on is speech.
  //  3. **Unknown is not empty.** A worker that cannot name its voices gets
  //     no picker, rather than an empty one implying it has none.
  //
  // The prefs key is the standalone page's, deliberately shared: one owner,
  // one box, one idea of what mecha sounds like — a voice picked on the
  // voice page is the voice this call opens with.
  const VOICE_PREFS = 'mecha.voice.prefs';
  let vCfg = $state(null); // the server's last answer: {voices, voice, speed, range}
  let vRefused = $state(false);
  let vRestored = false;

  function loadVoicePrefs() {
    // Storage can throw outright (private mode, blocked site data), so a
    // failed read degrades to "no preference" and never to a broken call.
    try {
      return JSON.parse(localStorage.getItem(VOICE_PREFS)) || {};
    } catch {
      return {};
    }
  }
  function saveVoicePrefs(p) {
    try {
      localStorage.setItem(VOICE_PREFS, JSON.stringify(p));
    } catch {
      /* not worth a word */
    }
  }

  function onVoiceConfig(cfg) {
    if (!cfg.voices || !cfg.voices.length) return; // unknown ≠ empty: stay hidden
    vCfg = cfg;
    vRefused = !!cfg.refused;
    if (cfg.refused) setTimeout(() => (vRefused = false), 1200);
    if (!vRestored) {
      vRestored = true;
      const p = loadVoicePrefs();
      const patch = {};
      if (p.voice && p.voice !== cfg.voice && cfg.voices.includes(p.voice)) patch.voice = p.voice;
      if (p.speed && Math.abs(p.speed - cfg.speed) > 0.01) patch.speed = p.speed;
      if (Object.keys(patch).length) vSession?.voiceConfig(patch);
    } else {
      saveVoicePrefs({ voice: cfg.voice, speed: cfg.speed });
    }
  }

  function setVoice(name) {
    vSession?.voiceConfig({ voice: name });
  }
  function setSpeed(value) {
    vSession?.voiceConfig({ speed: Number(value) });
  }

  // `keep` is the reconnect path: the words already spoken stay on screen,
  // because the call dropping is not the conversation ending — D3 means the
  // session outlived the transport, and clearing the pane would say
  // otherwise to the one person who just watched it fail.
  function startVoice({ keep = false } = {}) {
    // connect() inside the tap handler — the audio unlock needs the gesture.
    if (!keep) vEntries = [];
    vState = { name: 'connecting', label: 'connecting' };
    vSession = createVoiceSession({
      // Same-origin: serve proxies to the loopback runner, so the offer
      // rides the owner guard and no cross-origin fetch exists to fail.
      offerUrl: '/api/offer',
      // D3: the call is this conversation. Read at connect time rather
      // than bound reactively — switching sessions mid-call must not
      // silently redirect the words being spoken into a different one.
      sessionKey: key,
      onState: (name, label) => (vState = { name, label }),
      onTranscript,
      onLevel: (level) => (vLevel = level),
      onLink: (live) => {
        vLinked = live;
        if (!live) {
          // The controls describe a worker that is no longer there; a
          // picker left on screen after the line drops is a control that
          // silently stops doing anything.
          vCfg = null;
          vRestored = false;
        }
        if (!live && voiceOpen) {
          // Every idle label offers the same way back, because they are all
          // the same situation to the person looking at them: the call is
          // gone and the logo is how you get it again.
          vState = { name: 'idle', label: 'line dropped — tap the logo to reconnect' };
        }
      },
      onBotTurnEnd: () => {},
      onVoiceConfig,
    });
    vCfg = null;
    vRestored = false;
    voiceOpen = true;
    vSession.connect().catch((e) => {
      vState = {
        name: 'idle',
        label: `could not connect: ${e?.message ?? e} — tap the logo to try again`,
      };
    });
  }

  // The state label has said "tap to reconnect" since voice shipped and
  // nothing was listening: the logo is an <svg role="img">, so the sentence
  // described an affordance that did not exist. Ending the dead session
  // first is the part that is not just wiring — `startVoice` overwrites
  // `vSession`, so reconnecting without this leaves the previous peer
  // connection and its microphone track open for the life of the page.
  function reconnectVoice() {
    if (vState.name !== 'idle') return;
    try {
      vSession?.end();
    } catch {
      // Already dead is the normal case here; it is what we are recovering from.
    }
    vSession = null;
    startVoice({ keep: true });
  }

  let vMuted = $state(false);
  function toggleMute() {
    if (!vSession) return;
    vMuted = !vMuted;
    vSession.setMicEnabled(!vMuted);
  }

  function endVoice() {
    try {
      vSession?.end();
    } finally {
      vSession = null;
      voiceOpen = false;
      vLevel = 0;
    }
  }

  $effect(() => () => vSession?.end());

  async function send() {
    let text = draft.trim();
    if (attachments.length) {
      const lines = attachments.map((p) => `Attached file at ${p}`).join('\n');
      text = text ? `${text}\n\n${lines}` : lines;
      attachments = [];
    }
    if (!text) return;
    draft = '';
    try {
      const res = await fetch(`/api/chat/${key}/send`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ text }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      if (data.started) {
        pushEntry({ kind: 'user', text });
        running = true;
      } else if (data.steered) {
        pushEntry({ kind: 'user', text, queued: true });
      }
    } catch (e) {
      pushEntry({ kind: 'notice', text: `send failed: ${e?.message ?? e}` });
    }
  }

  // Phase 4's upload half: the file lands in the session jail's inbox/ and
  // the *path* is announced in the message — never the content, so the taint
  // arms through fs_read when the run opens it (the remote-control rule).
  let fileInput = $state(null);
  let uploading = $state(false);
  let attachments = $state([]); // workspace-relative paths, announced on send

  async function uploadPicked(e) {
    const files = [...(e.target.files ?? [])];
    e.target.value = '';
    for (const f of files) {
      uploading = true;
      try {
        const q = new URLSearchParams({ name: f.name });
        const res = await fetch(`/api/chat/${key}/upload?${q}`, { method: 'POST', body: f });
        if (!res.ok) throw new Error((await res.text()).trim());
        const data = await res.json();
        attachments.push(data.path);
      } catch (err) {
        pushEntry({ kind: 'notice', text: `upload failed: ${err?.message ?? err}` });
      } finally {
        uploading = false;
      }
    }
  }

  async function respond(entry, payload) {
    try {
      const res = await fetch(`/api/chat/${key}/answer`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ qid: entry.qid, ...payload }),
      });
      // 410 = answered elsewhere or expired; the card is stale either way.
      if (res.ok || res.status === 410) {
        entries = entries.filter((e) => e !== entry);
      }
    } catch {
      // leave the card; the timeout resolves it honestly server-side
    }
  }

  // Ascending order of what the run may do without asking. Cycling forward
  // rather than offering a menu keeps the control one tap on a phone; what
  // stops it being a trap is that the chip reads back the server's answer,
  // so a tap that did not land shows as a chip that did not move.
  const MODES = ['read_only', 'ask', 'allow'];
  const MODE_LABEL = { read_only: 'read-only', ask: 'ask', allow: 'allow' };

  // Entering `allow` asks; leaving it does not. Every other mode change is
  // one tap because it only ever *adds* a gate, and a confirmation on a
  // harmless change is what teaches people to tap through the ones that
  // matter. This one is a mis-tap away from the default posture and turns
  // off every approval for the session, so it is the exception.
  function nextMode() {
    const next = MODES[(MODES.indexOf(mode) + 1) % MODES.length];
    if (next === 'allow') {
      const ok = confirm(
        'Allow: tool calls run without asking, for this session until you change it.\n\n' +
          'Sends still stage in the outbox, and the interlock still refuses them once ' +
          'this conversation holds both private and outside content.'
      );
      if (!ok) return;
    }
    setMode(next);
  }

  async function setMode(next) {
    // Optimistic, so the chip moves under the thumb and a second tap cycles
    // from where the first left it — reading `mode` after the await made
    // two quick taps on a slow link compute the same next mode twice. The
    // server's own event is still what settles it; this only reverts a
    // change that never landed.
    const prev = mode;
    mode = next;
    try {
      const res = await fetch(`/api/chat/${key}/mode`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ mode: next }),
      });
      if (!res.ok) {
        mode = prev;
        pushEntry({ kind: 'notice', text: (await res.text()).trim() });
      }
    } catch {
      mode = prev;
    }
  }

  async function cancel() {
    try {
      await fetch(`/api/chat/${key}/cancel`, { method: 'POST' });
    } catch {
      // The done event reports the real outcome either way.
    }
  }

  const pct = $derived(
    usage?.window ? Math.min(100, Math.round((usage.prompt / usage.window) * 100)) : null
  );
  const fmt = (n) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));
</script>

<div class="chat">
  <header>
    <button class="menubtn" onclick={openDrawer} aria-label="sessions">
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M4 7h16M4 12h16M4 17h16" /></svg>
    </button>
    <span class="title">{key === 'main' ? 'Chat' : key}</span>
    <div class="meta">
      {#if taint?.untrusted || taint?.private}
        <span
          class="chip taint"
          title="what this conversation has touched decides what it may still do"
        >
          {taint.private ? 'private' : ''}{taint.private && taint.untrusted ? ' + ' : ''}{taint.untrusted ? 'untrusted' : ''}
        </span>
      {/if}
      <button
        class="chip modechip"
        class:ask={mode === 'ask'}
        class:allow={mode === 'allow'}
        onclick={nextMode}
        title="read-only: reads run, sends stage · ask: every other call becomes an approval card · allow: nothing asks (the interlock still refuses sends once this conversation holds private and untrusted content)"
      >{MODE_LABEL[mode] ?? mode}</button>
      <span class="chip">{model || '…'}</span>
    </div>
  </header>

  {#if drawer}
    <div class="scrim" onclick={() => (drawer = false)} aria-hidden="true"></div>
    <aside class="drawer">
      <div class="drawer-head">
        <span class="drawer-title">Sessions</span>
        <button class="newbtn" onclick={() => { drawer = false; newSession(); }}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M12 5v14M5 12h14" /></svg>
          new
        </button>
      </div>
      <div class="drawer-scroll">
        <div class="dsection">open</div>
        {#each rail.length ? rail : [{ key: 'main', running: false }] as s}
          <button class="drow" class:dactive={s.key === key} onclick={() => { drawer = false; switchTo(s.key); }}>
            <span class="raildot" class:on={s.running}></span>
            <span class="dname">{sessionLabel(s)}</span>
            {#if s.title?.startsWith('voice')}<span class="dkind">voice</span>{/if}
            {#if s.taint?.untrusted}<span class="railtaint">▲</span>{/if}
          </button>
        {/each}
        <div class="dsection">earlier</div>
        {#if history === null}
          <div class="dempty">reading the record…</div>
        {:else}
          {#each history.filter((h) => !h.attached_key) as h}
            <button class="drow past" onclick={() => resumeSession(h.id)}>
              <span class="dsnippet">{h.snippet}</span>
              <span class="dmeta">
                {#if h.kind === 'voice'}<span class="dkind">voice</span>{/if}
                {h.created_at.slice(0, 10)}
              </span>
            </button>
          {:else}
            <div class="dempty">nothing recorded yet</div>
          {/each}
        {/if}
      </div>
    </aside>
  {/if}

  <div class="transcript" bind:this={transcriptEl}>
    {#if error}
      <div class="notice">{error}</div>
    {/if}
    {#each entries as entry}
      {#if entry.kind === 'user'}
        <div class="bubble" class:queued={entry.queued}>
          {entry.text}
          {#if entry.queued}<span class="queued-tag">steered</span>{/if}
          {#if entry.spoken}<span class="queued-tag">spoken</span>{/if}
        </div>
      {:else if entry.kind === 'assistant'}
        <div class="answer">{entry.text}</div>
      {:else if entry.kind === 'tool'}
        <div class="tool" class:err={entry.is_error} class:blocked={entry.blocked}>
          <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 6l6 6-6 6" /></svg>
          <span>{entry.name}</span>
          {#if entry.pending}<span class="tool-state">running…</span>
          {:else if entry.blocked}<span class="tool-state">blocked (read-only)</span>
          {:else if entry.is_error}<span class="tool-state">failed</span>{/if}
        </div>
      {:else if entry.kind === 'notice'}
        <div class="notice">{entry.text}</div>
      {:else if entry.kind === 'draft'}
        {@const d = entry.draft}
        <div class="qcard dcard">
          <div class="qhead">
            <span class="qkicker">drafted — send it?</span>
            <span class="qtool">{d.label}</span>
          </div>
          <!-- The taint warning sits above everything, as it does in every
               other review surface: it is the one thing that changes how the
               rest should be read. -->
          {#if d.taint?.armed}
            <div class="dwarn">
              Written while third-party text was in this conversation — read the
              addressing carefully.
            </div>
          {/if}
          {#if d.headline}<div class="dheadline">{d.headline}</div>{/if}
          {#each d.headers as [name, value]}
            <div class="dfield"><span class="dkey">{name}</span><span>{value}</span></div>
          {/each}
          {#if d.body}<div class="dbody">{d.body}</div>{/if}
          {#each d.other as [name, value]}
            <div class="dfield"><span class="dkey">{name}</span><span>{value}</span></div>
          {/each}
          <!-- A reply's reviewable object includes what it replies to, and
               these bytes are third-party text: every line is marked, because
               a heading scrolls off and a per-line gutter cannot. -->
          {#if d.sources?.length}
            <button class="dtoggle" onclick={() => (entry.showSource = !entry.showSource)}>
              {entry.showSource ? 'hide' : 'show'} what this answers
            </button>
            {#if entry.showSource}
              {#each d.sources as src}
                <div class="dsrchead">{src.heading}</div>
                <div class="dsrc">{src.text}</div>
              {/each}
            {/if}
          {/if}
          {#if entry.error}<div class="dwarn">{entry.error}</div>{/if}
          <div class="qrow">
            <button class="qbtn" disabled={entry.busy} onclick={() => keepDraft(entry)}>
              Later
            </button>
            <button class="qbtn primary" disabled={entry.busy} onclick={() => releaseDraft(entry)}>
              {entry.busy ? 'sending…' : 'Send now'}
            </button>
          </div>
          <div class="qfoot">Later leaves it in the outbox — nothing here throws a draft away</div>
        </div>
      {:else if entry.kind === 'question' && entry.qkind === 'approval'}
        <div class="qcard">
          <div class="qhead">
            <span class="qkicker">mecha wants to run</span>
            <span class="qtool">{entry.tool}</span>
          </div>
          {#if entry.draft}
            <!-- Essentials first, the whole call one tap away. A card that
                 leads with a JSON blob is one people learn to approve
                 without reading, which is the outbox's rule arriving where
                 it was always needed. -->
            {#if entry.draft.headers.length}
              <dl class="qfields">
                {#each entry.draft.headers as [k, v]}
                  <dt>{k.replace(/_/g, ' ')}</dt>
                  <dd>{v}</dd>
                {/each}
              </dl>
            {/if}
            {#if entry.draft.body}<p class="qbody">{entry.draft.body}</p>{/if}
            <!-- After the body and never behind the toggle: `shell` has no
                 header or body field at all, so hiding `other` rendered an
                 empty card over `rm -rf build`. The expansion is for the
                 exact bytes, never for a field the reviewer needs. -->
            {#if entry.draft.other.length}
              <dl class="qfields">
                {#each entry.draft.other as [k, v]}
                  <dt>{k.replace(/_/g, ' ')}</dt>
                  <dd>{v}</dd>
                {/each}
              </dl>
            {/if}
            {#if entry.args}
              <button class="qmore" onclick={() => (entry.expanded = !entry.expanded)}>
                {entry.expanded ? 'less' : 'the whole call'}
              </button>
              {#if entry.expanded}<pre class="qargs">{entry.args}</pre>{/if}
            {/if}
          {:else if entry.args}
            <pre class="qargs">{entry.args}</pre>
          {/if}
          {#if entry.denying}
            <input
              class="qinput"
              placeholder="why not? (recorded, and learned from)"
              bind:value={entry.denyReason}
            />
            <div class="qrow">
              <button class="qbtn" onclick={() => (entry.denying = false)}>Back</button>
              <button
                class="qbtn deny"
                onclick={() => respond(entry, { allow: false, reason: entry.denyReason })}
              >Deny</button>
            </div>
          {:else}
            <div class="qrow">
              <button class="qbtn" onclick={() => (entry.denying = true)}>Deny…</button>
              <button class="qbtn primary" onclick={() => respond(entry, { allow: true })}>
                Allow
              </button>
            </div>
          {/if}
          <div class="qfoot">unanswered in 2m → refused as machine policy, never as your no</div>
        </div>
      {:else if entry.kind === 'question'}
        <div class="qcard">
          <div class="qhead">
            <span class="qkicker">mecha asks</span>
          </div>
          <div class="qtext">{entry.question}</div>
          {#if entry.options.length}
            <div class="qopts">
              {#each entry.options as option}
                <button class="qopt" onclick={() => respond(entry, { answer: option })}>
                  {option}
                </button>
              {/each}
            </div>
          {/if}
          <div class="qrow">
            <input
              class="qinput"
              placeholder="something else…"
              bind:value={entry.freeText}
              onkeydown={(e) => {
                if (e.key === 'Enter' && entry.freeText.trim()) {
                  respond(entry, { answer: entry.freeText.trim() });
                }
              }}
            />
            <button
              class="qbtn primary slim"
              disabled={!entry.freeText.trim()}
              onclick={() => respond(entry, { answer: entry.freeText.trim() })}
            >Answer</button>
          </div>
          <button class="qdecline" onclick={() => respond(entry, { decline: true })}>
            Decline — mecha proceeds without guessing
          </button>
        </div>
      {/if}
    {/each}
    {#if streaming}
      <div class="answer">{streaming}</div>
    {/if}
    {#if running && !streaming}
      <div class="thinking">
        <span class="dot"></span><span class="dot d2"></span><span class="dot d3"></span>
      </div>
    {/if}
  </div>

  <footer>
    {#if usage}
      <div class="gauge-row">
        <div class="gauge">
          <div
            class="fill"
            style:width="{pct ?? 0}%"
            style:background={pct !== null && pct >= 75 ? 'var(--hazard)' : 'var(--accent-400)'}
          ></div>
        </div>
        <span class="gauge-label">
          context {fmt(usage.prompt)}{usage.window ? ` / ${fmt(usage.window)}` : ''}
        </span>
      </div>
    {/if}
    {#if attachments.length}
      <div class="attach-row">
        {#each attachments as p, i}
          <button class="attach-chip" title="remove" onclick={() => attachments.splice(i, 1)}>
            {p.split('/').pop()} ✕
          </button>
        {/each}
      </div>
    {/if}
    <div class="input-row">
      <input type="file" multiple hidden bind:this={fileInput} onchange={uploadPicked} />
      <button class="round" disabled={uploading} onclick={() => fileInput?.click()} title="attach a file — it lands in this session's inbox/">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="var(--accent-400)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.5l-8.2 8.2a5.5 5.5 0 01-7.8-7.8L13.6 4.3a3.7 3.7 0 015.2 5.2l-8.4 8.4a1.85 1.85 0 01-2.6-2.6l7.8-7.8" /></svg>
      </button>
      <textarea
        rows="1"
        placeholder={running ? 'Steer the run…' : 'Ask mecha…'}
        bind:value={draft}
        onkeydown={(e) => {
          if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            send();
          }
        }}
      ></textarea>
      <button
        class="round voice"
        onclick={startVoice}
        title="start a voice call in this conversation"
      >
        <svg viewBox="0 0 24 24" width="19" height="19" fill="none" stroke="var(--accent-400)" stroke-width="1.8" stroke-linecap="round"><path d="M4 10v4M8 7v10M12 4v16M16 7v10M20 10v4" /></svg>
      </button>
      {#if running}
        <button class="round stop" onclick={cancel} title="stop at the next safe point">
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round"><rect x="7" y="7" width="10" height="10" rx="1.5" /></svg>
        </button>
      {/if}
      <button class="round send" onclick={send} title="send">
        <svg viewBox="0 0 24 24" width="19" height="19" fill="none" stroke="var(--void)" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M12 19V5M6 11l6-6 6 6" /></svg>
      </button>
    </div>
  </footer>

  {#if voiceOpen}
    <div class="voice-overlay">
      <div class="voice-top">
        <span class="chip">speaking into {key === 'main' ? 'your chat' : `“${key}”`} — same conversation, same memory</span>
      </div>
      <div class="voice-stage">
        <!-- A button, not decoration: the idle label tells people to tap this
             to get the call back, so it has to be the thing that does it. It
             is only live when idle, or a tap mid-call would tear down a
             working line. -->
        <button
          class="logo"
          class:tappable={vState.name === 'idle'}
          disabled={vState.name !== 'idle'}
          onclick={reconnectVoice}
          aria-label={vState.name === 'idle' ? 'reconnect the call' : `mecha ${vState.label}`}
        >
          <svg viewBox="0 0 63 54" width="112" height="96" aria-hidden="true">
            <g fill="var(--accent-700)">
              <path d="M0 0h24l7.5 8.5L39 0h24v16H0z" />
              <path d="M0 20h14v15H0zM49 20h14v15H49zM0 39h14v15H0zM49 39h14v15H49z" />
              <path d="M14 39v15h13.24zM49 39v15H35.76z" />
            </g>
            <path
              d="M21 24h21v7H21z"
              class="slot {vState.name}"
              style:opacity={vState.name === 'listening' ? 0.7 + vLevel * 0.3 : 1}
            />
          </svg>
        </button>
        <div class="voice-state">
          <span class="vdot" class:live={vLinked}></span>
          <span>{vState.label}</span>
        </div>
        <div class="meter" title="your microphone, live">
          {#each Array(14) as _, i}
            <span
              class="tick"
              class:lit={vLevel * 14 > i}
              style:height="{6 + Math.abs(i - 6.5) * -0 + (i % 2 ? 6 : 0) + 8}px"
            ></span>
          {/each}
        </div>
      </div>
      <div class="voice-pane" bind:this={voicePane}>
        {#each vEntries as entry}
          {#if entry.who === 'user'}
            <div class="vbubble" class:interim={entry.interim}>{entry.text}</div>
          {:else}
            <div class="vanswer" class:interim={entry.interim}>{entry.text}</div>
          {/if}
        {/each}
      </div>
      {#if vCfg}
        <div class="voice-settings" class:refused={vRefused}>
          <label class="vset">
            <span class="vlabel">voice</span>
            <select
              class="vselect"
              value={vCfg.voice}
              onchange={(e) => setVoice(e.currentTarget.value)}
            >
              {#each vCfg.voices as v}
                <option value={v}>{v}</option>
              {/each}
            </select>
          </label>
          <label class="vset">
            <span class="vlabel">rate</span>
            <!-- The range is the server's, never a literal here: the worker
                 owns what it can speak at, and a hardcoded bound would be a
                 second opinion about it. -->
            <input
              class="vspeed"
              type="range"
              min={vCfg.range?.min ?? 0.5}
              max={vCfg.range?.max ?? 2}
              step="0.05"
              value={vCfg.speed}
              onchange={(e) => setSpeed(e.currentTarget.value)}
            />
            <span class="vspeedval">{Number(vCfg.speed).toFixed(2)}×</span>
          </label>
        </div>
      {/if}
      <div class="voice-controls">
        <button class="mutebtn" class:muted={vMuted} onclick={toggleMute} title={vMuted ? 'unmute' : 'mute'}>
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <rect x="9" y="3" width="6" height="11" rx="3" />
            <path d="M5 11a7 7 0 0014 0M12 18v3" />
            {#if vMuted}<path d="M4 4l16 16" />{/if}
          </svg>
        </button>
        <button class="endcall" onclick={endVoice} title="end the call">
          <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="var(--hazard)" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M6 6l12 12M18 6L6 18" /></svg>
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  .chat {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 22px 20px 6px;
  }
  .title {
    font-weight: 500;
    font-size: 17px;
    letter-spacing: -0.02em;
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .chip.taint {
    color: var(--hazard);
  }
  .menubtn {
    background: none;
    border: none;
    color: var(--text-muted);
    min-width: 44px;
    min-height: 44px;
    margin: -10px 4px -10px -12px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .scrim {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    z-index: 40;
  }
  .drawer {
    position: fixed;
    top: 0;
    left: 0;
    bottom: 0;
    width: min(320px, 85vw);
    background: var(--bg);
    border-right: 1px solid var(--accent-700);
    z-index: 41;
    display: flex;
    flex-direction: column;
    padding-top: env(safe-area-inset-top);
    animation: drawer-in 0.18s ease-out;
  }
  @keyframes drawer-in {
    from { transform: translateX(-100%); }
    to { transform: translateX(0); }
  }
  .drawer-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 18px 16px 12px;
    border-bottom: 1px solid var(--accent-900);
  }
  .drawer-title {
    font-weight: 500;
    font-size: 16px;
    letter-spacing: -0.02em;
  }
  .newbtn {
    display: flex;
    align-items: center;
    gap: 5px;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text);
    background: var(--accent-900);
    border: 1px solid var(--accent-700);
    border-radius: var(--radius-chip);
    padding: 8px 12px;
    min-height: 38px;
    cursor: pointer;
  }
  .drawer-scroll {
    flex: 1;
    overflow-y: auto;
    padding: 8px 8px calc(12px + env(safe-area-inset-bottom));
  }
  .drawer-scroll > * { flex-shrink: 0; }
  .dsection {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--accent-700);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    padding: 12px 10px 6px;
  }
  .drow {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    border-radius: var(--radius);
    color: var(--text);
    font: inherit;
    padding: 11px 10px;
    min-height: 44px;
    cursor: pointer;
  }
  .drow.dactive {
    background: var(--accent-900);
  }
  .drow.past {
    flex-direction: column;
    align-items: stretch;
    gap: 4px;
  }
  .dname {
    font-family: var(--mono);
    font-size: 13px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dsnippet {
    font-size: 13px;
    line-height: 1.4;
    color: var(--text);
    overflow: hidden;
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    overflow-wrap: anywhere;
  }
  .dmeta {
    display: flex;
    align-items: center;
    gap: 6px;
    font-family: var(--mono);
    font-size: 10px;
    color: var(--text-muted);
  }
  .dkind {
    font-family: var(--mono);
    font-size: 9px;
    color: var(--accent-400);
    background: var(--accent-900);
    border-radius: var(--radius-chip);
    padding: 2px 6px;
  }
  .dempty {
    font-size: 12px;
    color: var(--text-muted);
    padding: 10px;
  }
  .raildot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--accent-900);
  }
  .raildot.on {
    background: var(--accent-400);
  }
  .railtaint {
    color: var(--hazard);
    font-size: 9px;
  }
  .transcript {
    flex: 1;
    overflow-y: auto;
    padding: 16px 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .bubble {
    align-self: flex-end;
    max-width: 82%;
    background: var(--surface);
    border-radius: var(--radius);
    padding: 11px 14px;
    font-size: 14px;
    line-height: 1.45;
    white-space: pre-wrap;
  }
  .bubble.queued {
    border: 1px solid var(--accent-700);
    background: var(--bg);
  }
  .queued-tag {
    display: block;
    margin-top: 4px;
    font-family: var(--mono);
    font-size: 9px;
    color: var(--text-muted);
  }
  .answer {
    max-width: 92%;
    font-size: 14px;
    line-height: 1.5;
    white-space: pre-wrap;
  }
  .tool {
    display: flex;
    align-items: center;
    gap: 7px;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-muted);
  }
  .tool svg {
    color: var(--accent-700);
  }
  .tool-state {
    font-size: 11px;
    color: var(--accent-700);
  }
  .tool.err .tool-state,
  .tool.blocked .tool-state {
    color: var(--hazard);
  }
  .notice {
    font-size: 12px;
    color: var(--hazard);
    display: flex;
    gap: 8px;
  }
  .modechip {
    cursor: pointer;
    background: var(--bg);
    min-height: 28px;
  }
  .modechip.ask {
    color: var(--accent-100);
    background: var(--accent-900);
    border-color: var(--accent-500);
  }
  /* Hazard is a signal here, and per brand.md it stays text and a thin line
     — never an area fill. `allow` is the one mode where nothing will stop
     to ask, so it is the one chip that should catch the eye across a room. */
  .modechip.allow {
    color: var(--hazard);
    background: var(--bg);
    border-color: var(--hazard);
  }
  .qcard {
    background: var(--surface);
    border: 1px solid var(--accent-500);
    border-radius: var(--radius);
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .dcard {
    gap: 8px;
  }
  .dwarn {
    font-size: 12px;
    line-height: 1.45;
    color: var(--hazard);
    border-left: 2px solid var(--hazard);
    padding-left: 10px;
  }
  .dheadline {
    font-size: 15px;
    font-weight: 500;
    line-height: 1.35;
  }
  .dfield {
    display: flex;
    gap: 8px;
    font-size: 13px;
    line-height: 1.45;
  }
  .dkey {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    min-width: 62px;
    flex-shrink: 0;
    padding-top: 3px;
  }
  .dbody {
    font-size: 14px;
    line-height: 1.55;
    white-space: pre-wrap;
    padding: 8px 0;
    border-top: 1px solid var(--accent-900);
    border-bottom: 1px solid var(--accent-900);
  }
  .dtoggle {
    background: none;
    border: none;
    padding: 0;
    color: var(--text-muted);
    font-family: var(--mono);
    font-size: 11px;
    text-align: left;
    cursor: pointer;
  }
  .dsrchead {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--text-muted);
  }
  /* Third-party text, marked on every line: a heading scrolls off, a gutter
     cannot. */
  .dsrc {
    font-size: 13px;
    line-height: 1.5;
    white-space: pre-wrap;
    color: var(--text-muted);
    border-left: 2px solid var(--accent-900);
    padding-left: 10px;
  }
  .qhead {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .qkicker {
    font-family: var(--mono);
    font-size: 10px;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .qtool {
    font-family: var(--mono);
    font-size: 12px;
    color: var(--accent-400);
  }
  .qtext {
    font-size: 15px;
    font-weight: 500;
    line-height: 1.4;
  }
  .qfields {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 2px 12px;
    margin: 0;
    font-size: 13px;
  }
  .qfields dt {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--text-muted);
    text-transform: lowercase;
    align-self: baseline;
  }
  .qfields dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .qbody {
    margin: 0;
    font-size: 14px;
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .qmore {
    align-self: flex-start;
    background: none;
    border: none;
    padding: 0;
    font-family: var(--mono);
    font-size: 10px;
    color: var(--accent-400);
    cursor: pointer;
  }
  .qargs {
    background: var(--void);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius-chip);
    padding: 10px;
    font-family: var(--mono);
    font-size: 11px;
    line-height: 1.5;
    overflow-x: auto;
    max-height: 180px;
    margin: 0;
  }
  .qopts {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .qopt {
    text-align: left;
    background: var(--bg);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius);
    color: var(--text);
    font-size: 14px;
    padding: 12px 14px;
    min-height: 48px;
    cursor: pointer;
  }
  .qopt:active {
    border-color: var(--accent-500);
  }
  .qrow {
    display: flex;
    gap: 8px;
  }
  .qbtn {
    flex: 1;
    min-height: 44px;
    background: var(--bg);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius);
    color: var(--text);
    font-size: 14px;
    cursor: pointer;
  }
  .qbtn.primary {
    background: var(--accent-400);
    color: var(--void);
    font-weight: 500;
    border: none;
  }
  .qbtn.deny {
    color: var(--hazard);
    border-color: var(--accent-700);
  }
  .qbtn.slim {
    flex: 0 0 88px;
  }
  .qbtn:disabled {
    opacity: 0.5;
  }
  .qinput {
    flex: 1;
    background: var(--void);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius);
    color: var(--text);
    font-size: 14px;
    padding: 11px 12px;
    min-height: 44px;
    box-sizing: border-box;
  }
  .qdecline {
    background: none;
    border: none;
    color: var(--text-muted);
    font-size: 13px;
    min-height: 44px;
    cursor: pointer;
  }
  .qfoot {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--text-muted);
  }
  .thinking {
    display: flex;
    gap: 5px;
    padding: 4px 2px;
  }
  .dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--accent-400);
    animation: pulse 1.2s infinite;
  }
  .d2 {
    animation-delay: 0.2s;
    background: var(--accent-500);
  }
  .d3 {
    animation-delay: 0.4s;
    background: var(--accent-700);
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 0.35;
    }
    50% {
      opacity: 1;
    }
  }
  footer {
    border-top: 1px solid var(--accent-900);
    background: var(--bg);
    padding: 10px 14px 8px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .gauge-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 4px;
  }
  .gauge {
    flex: 1;
    height: 3px;
    background: var(--accent-900);
    border-radius: 2px;
    overflow: hidden;
  }
  .fill {
    height: 3px;
  }
  .gauge-label {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--text-muted);
  }
  .attach-row { display: flex; gap: 6px; flex-wrap: wrap; padding: 0 0 8px; }
  .attach-chip { font-family: var(--mono); font-size: 11px; color: var(--text); background: var(--accent-900); border: 1px solid var(--accent-700); border-radius: var(--radius-chip); padding: 6px 10px; cursor: pointer; }
  .input-row {
    display: flex;
    align-items: flex-end;
    gap: 8px;
  }
  textarea {
    flex: 1;
    min-height: 44px;
    max-height: 130px;
    background: var(--surface);
    border: none;
    border-radius: var(--radius);
    padding: 12px 14px;
    color: var(--text);
    font-family: var(--sans);
    font-size: 16px;
    resize: none;
  }
  textarea:focus {
    outline: 1px solid var(--accent-500);
  }
  .round {
    width: 44px;
    height: 44px;
    border-radius: var(--radius);
    border: none;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }
  .send {
    background: var(--accent-400);
  }
  .stop {
    background: var(--surface);
    color: var(--hazard);
  }
  .voice {
    background: var(--surface);
    border: 1px solid var(--accent-700);
  }
  .voice-settings {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 18px;
    padding: 0 20px 10px;
    flex-wrap: wrap;
  }
  .vset {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .vlabel {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
  .vselect {
    background: var(--surface);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius-chip);
    color: var(--text);
    font-family: var(--mono);
    font-size: 12px;
    padding: 8px 10px;
    min-height: 38px;
    cursor: pointer;
  }
  .vspeed {
    -webkit-appearance: none;
    appearance: none;
    width: 96px;
    height: 3px;
    border-radius: 2px;
    background: var(--accent-900);
    cursor: pointer;
  }
  .vspeed::-webkit-slider-thumb {
    -webkit-appearance: none;
    width: 15px;
    height: 15px;
    border-radius: 50%;
    background: var(--accent-400);
    border: none;
    cursor: pointer;
  }
  .vspeed::-moz-range-thumb {
    width: 15px;
    height: 15px;
    border-radius: 50%;
    background: var(--accent-400);
    border: none;
    cursor: pointer;
  }
  .vspeedval {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-muted);
    min-width: 3.4em;
  }
  /* A refused value flashes rather than silently reverting: the control
     snapped back to the server's answer, and the owner should see that it
     did instead of wondering whether the drag registered. */
  .voice-settings.refused .vspeedval,
  .voice-settings.refused .vselect {
    color: var(--hazard);
    border-color: var(--hazard);
  }
  .logo {
    background: none;
    border: none;
    padding: 0;
    display: block;
    /* The disabled state is the ordinary one — mid-call this is a picture,
       and it must look exactly as it did before it became a button. */
    opacity: 1;
  }
  .logo:disabled {
    cursor: default;
  }
  .logo.tappable {
    cursor: pointer;
  }
  .voice-overlay {
    position: absolute;
    inset: 0;
    background: var(--void);
    display: flex;
    flex-direction: column;
    z-index: 5;
  }
  .voice-top {
    display: flex;
    justify-content: center;
    padding: 22px 20px 0;
  }
  .voice-stage {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 18px;
  }
  .slot {
    fill: var(--accent-700);
  }
  .slot.listening {
    fill: var(--accent-400);
  }
  .slot.thinking {
    fill: var(--accent-500);
    animation: slotpulse 1.1s infinite;
  }
  .slot.speaking {
    fill: var(--accent-300);
  }
  @keyframes slotpulse {
    0%,
    100% {
      opacity: 0.45;
    }
    50% {
      opacity: 1;
    }
  }
  .voice-state {
    display: flex;
    align-items: center;
    gap: 8px;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-muted);
  }
  .meter {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 24px;
  }
  .tick {
    width: 3px;
    border-radius: 1px;
    background: var(--accent-900);
    transition: background 60ms linear;
  }
  .tick.lit {
    background: var(--accent-400);
  }
  .vdot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--accent-900);
  }
  .vdot.live {
    background: var(--accent-400);
  }
  .voice-pane {
    max-height: 34%;
    overflow-y: auto;
    margin: 0 20px;
    background: var(--bg);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius);
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .vbubble {
    align-self: flex-end;
    max-width: 84%;
    background: var(--surface);
    border-radius: var(--radius);
    padding: 9px 12px;
    font-size: 13px;
    line-height: 1.5;
  }
  .vanswer {
    align-self: flex-start;
    max-width: 92%;
    font-size: 13px;
    line-height: 1.5;
  }
  .vbubble.interim,
  .vanswer.interim {
    color: var(--text-muted);
  }
  .voice-controls {
    display: flex;
    justify-content: center;
    align-items: center;
    gap: 24px;
    padding: 24px 0 34px;
  }
  .mutebtn {
    width: 56px;
    height: 56px;
    border-radius: 14px;
    background: var(--surface);
    border: 1px solid var(--accent-900);
    color: var(--text);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }
  .mutebtn.muted {
    color: var(--hazard);
    border-color: var(--accent-700);
  }
  .endcall {
    width: 68px;
    height: 68px;
    border-radius: 16px;
    background: var(--surface);
    border: 1px solid var(--accent-700);
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
  }

  /* Carried from the standalone voice page when it was retired: it had the
     only reduced-motion handling in either shell, and the animations that
     most need it are here rather than there. The two infinite ones are the
     concern - a perpetually pulsing dot is the classic vestibular trigger,
     and both of them encode state (thinking, speaking) that must survive
     the animation being switched off. So they degrade to a static colour
     rather than simply stopping, which would leave the state invisible. */
  @media (prefers-reduced-motion: reduce) {
    .drawer,
    .dot,
    .slot.thinking {
      animation: none !important;
    }
    .slot.thinking {
      fill: var(--accent-300);
    }
  }
</style>
