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
  // The agent's plan for this session, live. Rendered rather than summarised:
  // a paraphrase of a plan is a different plan, and this is the one part of a
  // long run that says how far it got rather than what is true.
  let todo = $state([]);
  // The board task this conversation is about, and whether a hand-over is in
  // flight. A task chat is the same chat with a subject.
  let task = $state(null);
  let handing = $state(false);
  let handNote = $state(null);
  let todoOpen = $state(true);
  const MARK = { completed: '[x]', in_progress: '[~]', pending: '[ ]' };
  let taint = $state(null);
  // §6.2's readout — the logo's tint. `null` means "nothing to say", which
  // is the overwhelming common case (the server sends an event only when
  // the label is not neutral, to avoid a wire event on every turn saying
  // nothing); `sawAffectThisRun` is what tells `done` whether to fall back
  // to that silence for a run that produced no event.
  let affect = $state(null);
  // The dimensional half of the same readout: `{positive, negative,
  // positives, negatives, visible, partial?}`, or null when the run had
  // nothing signed. Drawn as a two-sided bar by the owner's ruling for this
  // surface (APPRAISAL-RESEARCH §3.1); the TUI shows the same numbers as
  // text.
  let valence = $state(null);
  // Bar geometry: each side is its magnitude over this cap, clamped. The
  // record's steps are ±0.5/±1.0 per error, so three is "several".
  const VALENCE_CAP = 3;
  const barWidth = (m) => `${Math.min(m / VALENCE_CAP, 1) * 100}%`;
  let sawAffectThisRun = false;
  let usage = $state(null);
  let model = $state('');
  let draft = $state('');
  let error = $state(null);
  let transcriptEl = $state(null);
  let inputEl = $state(null);

  // Interim voice-out: the browser's own synthesis reads replies aloud when
  // toggled. Deliberately a stopgap — the real voice mode (Pipecat, the
  // chosen launch voice, barge-in) replaces this when the speech servers
  // land; until then it is the fail-to-a-lesser-mode shape, and marked so.

  // Which argument names a call, most specific first.
  //
  // `DraftView` is a *shape*, not a ranking: it lifts addressing into
  // `headers` and prose into `body`, and everything else falls through to
  // `other` in `serde_json::Map` order — which is `BTreeMap` order, because
  // `preserve_order` is off. So `other` arrives sorted by key, and the first
  // entry is the alphabetically first argument rather than the one a reader
  // would recognise the call by: `fs_read {path, offset, limit}` leads with
  // `limit`, `web_search {query, limit}` leads with `limit`. Two reads of
  // different files with the same limit are the same row twice, which is the
  // bug this chip exists to fix, wearing a confident label.
  //
  // Ranking here rather than in `DraftView` keeps that type a shape for
  // every one of its readers. Checked by `web/test/tool-digest.mjs`.
  const DIGEST_FIELDS = ['path', 'command', 'query', 'url', 'pattern', 'task', 'name', 'id'];

  // Header arguments that name the *store* rather than the call. `account`
  // is a `HEADER_FIELDS` member, so on every item-scoped mail and calendar
  // tool — `mail_get_thread {thread_id, account}`, `mail_reply`,
  // `mail_triage`, `calendar_delete_event` — it is the only header present
  // and would win outright, labelling three different threads `personal`.
  // It is required whenever several accounts are configured, so that is the
  // ordinary case here, not a corner: the argument shared by every call in
  // the turn is the one argument that cannot tell two of them apart. Kept as
  // a last resort below, because naming the account still beats naming
  // nothing.
  const SHARED_FIELDS = ['account'];

  // A bare number or boolean never says *which* call this was — it says how
  // much, how deep, how many. Values reach the page already rendered to
  // strings, so the type is gone and the shape is all that is left to go on.
  const QUANTITY = /^(-?\d+(\.\d+)?|true|false|null)$/;

  // One line saying *which* call this was, for the closed chip.
  //
  // Addressing first, minus the shared scope — `DraftView` ordered `headers`
  // for a reader already. Then a known identifying argument, then anything
  // ending in `_id`, then any argument that is not a bare quantity, and only
  // then the first one there is: an unanticipated tool still gets a label,
  // which is the fallback `other` has always been. Display only; the whole
  // call is one tap below, and nothing here decides anything.
  function toolDigest(draft) {
    if (!draft) return '';
    const other = draft.other ?? [];
    const headers = draft.headers ?? [];
    const pair =
      headers.find(([name]) => !SHARED_FIELDS.includes(name)) ??
      DIGEST_FIELDS.map((k) => other.find(([name]) => name === k)).find(Boolean) ??
      other.find(([name]) => name.endsWith('_id')) ??
      other.find(([, value]) => !QUANTITY.test(String(value).trim())) ??
      other[0] ??
      headers[0];
    return oneLine(pair ? pair[1] : (draft.body ?? ''));
  }

  function oneLine(text) {
    const flat = String(text).replace(/\s+/g, ' ').trim();
    return flat.length > 72 ? `${flat.slice(0, 72)}…` : flat;
  }

  // Which open chip a result or a refusal belongs to.
  //
  // **By id where there is one.** `Agent::run_tools` emits every `ToolCall`
  // in one sequential loop and only then runs the approved calls in a
  // `join_all`, which preserves order — so a turn holds several rows open at
  // once and their results come back in *call* order. Neither position nor
  // name pairs them: two `fs_write` calls in a turn got each other's output,
  // and a turn that refuses one call and runs another landed the refusal on
  // the row still running. Both were anonymous swaps until this row carried
  // the arguments, and confident mislabels afterwards — the failure this
  // change exists to prevent, arriving one layer above it.
  //
  // An id that matches nothing means the row is already closed, so the
  // result is dropped rather than moved onto somebody else's: the
  // planning-phase refusal emits `ToolDenied` *and* `ToolResult`, and the
  // denial already wrote the reason where it belongs.
  //
  // The name path is for events that genuinely have no id — `ToolDenied`
  // carries none — and for a `web/dist` older than the binary serving it,
  // which is the compatibility rule this file already follows elsewhere.
  function openCall(entries, ev) {
    const matches =
      ev.id == null ? (e) => e.name === ev.name : (e) => e.id === ev.id;
    return entries.findLast((e) => e.kind === 'tool' && e.pending && matches(e));
  }

  // A denial is the *end* of the call above it, not a second call.
  //
  // Three of the four denial paths in `Agent::run_tools` — the trifecta
  // interlock, a `pre_tool` hook deny, and the approver — emit `ToolDenied`
  // and write the tool-result block straight into `results[i]` with no
  // `AgentEvent::ToolResult` behind it. (Only the planning-phase refusal
  // emits both.) So nothing else will ever resolve the chip the `tool` event
  // opened: pushing a second entry left the first one `pending` for the rest
  // of the session. That was an inert row before the call carried `args`;
  // now the row is a working disclosure, and opening it asserted "still
  // running" over a call the interlock had already refused — the harness
  // rendering its own guard's refusal as work in flight.
  //
  // It also settles a disagreement between the two renderings: the reload
  // path sees the recorded result and draws *one* chip for this call, so a
  // live view drawing two was the transcript contradicting itself.
  function resolveDenial(entries, ev) {
    const open = openCall(entries, ev);
    if (!open) return false;
    open.pending = false;
    open.blocked = true;
    open.preview = ev.reason ?? '';
    return true;
  }

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

  async function refreshTodo() {
    try {
      const res = await fetch(`/api/chat/${key}`);
      if (!res.ok) return;
      todo = (await res.json()).todo ?? [];
    } catch {
      // A plan that failed to refresh is stale, not wrong, and saying so
      // in the transcript would be noise about the UI rather than the run.
    }
  }

  // **Let it carry on without you.** The conversation moves from this
  // process — where a question is a card and the run dies with a restart —
  // into a detached child, where a question ends the run and waits in the
  // store until morning. Not more capable: differently absent, which is a
  // fact about the owner rather than about the run.
  async function handOver() {
    if (handing || !task?.id) return;
    handing = true;
    handNote = null;
    try {
      const res = await fetch('/api/tasks/handover', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ task: task.id }),
      });
      if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      // The conversation is no longer this process's to append to, so the
      // page must stop acting as though it were. The board is where it is
      // watched from now.
      handNote = 'handed over — it carries on from here';
      location.hash = 'tasks';
    } catch (e) {
      handNote = String(e?.message ?? e);
    } finally {
      handing = false;
    }
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
      // What this conversation is about, when it is about a board task.
      // Absent for an ordinary chat, which renders exactly as before.
      task = data.task ?? null;
      todo = data.todo ?? [];
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
          // `draft` and `args` arrive with the call, so a run in flight is
          // as readable as one being re-read — the chip can say which file
          // it is writing while it writes it.
          pushEntry({
            kind: 'tool',
            name: ev.name,
            id: ev.id ?? null,
            draft: ev.draft ?? null,
            args: ev.args ?? null,
            pending: true,
          });
          break;
        case 'tool_result': {
          // A result whose chip is already closed is dropped, not moved onto
          // somebody else's row: the planning-phase refusal emits both
          // `ToolDenied` and `ToolResult`, and the denial already wrote the
          // reason where it belongs.
          const open = openCall(entries, ev);
          if (open) {
            open.pending = false;
            open.is_error = ev.is_error;
            open.preview = ev.preview;
          }
          // Every plan change already arrives here as a tool call, so the
          // list needs no event of its own — re-read on the one that means
          // it changed. Cheap while a run holds the conversation, because
          // the transcript read returns no entries then.
          if (ev.name === 'todo' && !ev.is_error) refreshTodo();
          break;
        }
        case 'denied':
          // The fallback stays: a refusal with no call above it is still a
          // refusal, and dropping it would be the quietest failure here.
          if (!resolveDenial(entries, ev)) {
            pushEntry({
              kind: 'tool',
              name: ev.name,
              blocked: true,
              pending: false,
              preview: ev.reason ?? '',
            });
          }
          break;
        case 'usage':
          usage = { prompt: ev.prompt_tokens, window: ev.context_window };
          break;
        case 'notice':
          pushEntry({ kind: 'notice', text: ev.text });
          break;
        case 'affect':
          // `neutral` is a label saying nothing; the event is sent for its
          // valence in that case, and the chip shows the numbers alone.
          // The server omits `label` when it says nothing; the `neutral`
          // guard stays for a binary that predates the omission.
          affect = ev.label && ev.label !== 'neutral' ? ev.label : null;
          valence = ev.valence && (ev.valence.positives || ev.valence.negatives) ? ev.valence : null;
          sawAffectThisRun = true;
          break;
        case 'titled':
          // The conversation has a name now. Reload the rail rather than
          // writing the name straight into the header: the rail is where
          // every surface reads a session's name, and two paths to one
          // label is how a header and a drawer row start disagreeing.
          loadRail();
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
          // A run that produced no `affect` event was `Neutral` — the
          // server never says so out loud, so silence is read as such here.
          // Reset here rather than at run start: `WireEvent::Affect` is
          // always sent before `Done` within one `begin_turn`, so by the
          // time this fires the flag has already done its job for this
          // run — and resetting only at run-start events (`data.started`,
          // `'user'`) missed a second tab observing a *typed* turn driven
          // from elsewhere, which emits neither: that tab's tint from an
          // earlier run never cleared. Resetting here covers every
          // observer, not just the one that sent the turn.
          if (!sawAffectThisRun) {
            affect = null;
            valence = null;
          }
          sawAffectThisRun = false;
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
    // Same rule as everywhere else this readout guards against staleness
    // (the TUI's `/clear`, voice's `Hosted::Unknown` fall-through): the
    // tint describes the *previous* conversation's last run, and nothing
    // else here would clear it — `/api/chat/{key}` carries no affect
    // field, and the new key's own SSE subscription emits `Affect` only
    // once a run there finishes.
    affect = null;
    valence = null;
    sawAffectThisRun = false;
  }

  // The drawer: every conversation this process holds, and the recorded
  // ones from earlier — the pattern every multi-session app converges on,
  // a left panel that expands and collapses. Voice sessions are here too:
  // a brainstorm spoken on a walk resumes as a text chat, same
  // conversation, same taint.
  let drawer = $state(false);
  let history = $state(null);
  /// Wide enough that the drawer stops being a drawer and simply stays
  /// open. Read from `matchMedia` rather than inferred from anything: the
  /// docked panel and the overlay one are the same markup, and only one of
  /// them wants a scrim, an animation and a tap-to-close.
  let docked = $state(false);
  $effect(() => {
    const mq = window.matchMedia('(min-width: 1180px)');
    const apply = () => {
      docked = mq.matches;
      if (docked) {
        drawer = false;
        loadRail();
        loadHistory();
      }
    };
    apply();
    mq.addEventListener('change', apply);
    return () => mq.removeEventListener('change', apply);
  });

  /// This conversation's own row in the rail, which is where its title
  /// lives — the header renders the same name the drawer does, from the
  /// same source, so they cannot disagree.
  const heading = $derived.by(() => {
    // A key minted a moment ago is not in the rail yet; showing it while we
    // wait would put `chat-8f3a` in the header of a conversation that is
    // about to be called something, which is the thing this replaced.
    const row = rail.find((s) => s.key === key);
    const name = row ? sessionLabel(row) : key === DEFAULT_KEY ? DEFAULT_KEY : 'new chat';
    if (key === DEFAULT_KEY && name === DEFAULT_KEY) return 'Chat';
    return name === 'new chat' ? 'New chat' : name;
  });

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

  // A session id in the route (`#chat/<id>`), from the board's "open the
  // conversation". Resumed once: the endpoint returns the live key of a
  // session this process already holds rather than minting a twin, so a
  // second attempt would be harmless — but a re-run on every route change
  // would fight the user switching sessions by hand.
  let { resume = null } = $props();
  let resumed = $state(null);
  $effect(() => {
    if (resume && resume !== resumed) {
      resumed = resume;
      resumeSession(resume);
    }
  });

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

  /// What to call a conversation. The server titles a session from the
  /// owner's own opening turns once there are some (`web: <summary>`);
  /// until then the recorded title is still the minted key, which is an
  /// address rather than a name — so say so, instead of showing `chat-8f3a`
  /// as if it meant something.
  /// The stored form carries which door a session came through (`web: `,
  /// `voice: `, `task: `); that is a storage detail and a `kind` field of
  /// its own, never part of the name.
  const nameOf = (title) => (title ?? '').replace(/^(web|voice|task): /, '').trim();

  const sessionLabel = (s) => {
    const t = nameOf(s.title);
    if (!t || t === s.key) return s.key === DEFAULT_KEY ? DEFAULT_KEY : 'new chat';
    return t;
  };

  const DEFAULT_KEY = 'main';

  /// **A new conversation asks nothing.** It used to ask for a name — a
  /// modal prompt, a lowercase-and-dashes rule, and a decision to make
  /// before saying the thing you opened the app to say. The key is minted
  /// here (it is an address: it has to be unique and URL-safe, and nothing
  /// else), and the *name* arrives from the conversation itself once there
  /// is one to summarise.
  function newSession() {
    // Already sitting in an empty one: opening a second would leave a
    // trail of blank sessions behind a button people press to clear their
    // head. Nothing to do but put the cursor where they expect it.
    if (!entries.length && !running) {
      drawer = false;
      queueMicrotask(() => inputEl?.focus());
      return;
    }
    const k = `chat-${Math.random().toString(36).slice(2, 8)}`;
    drawer = false;
    switchTo(k);
    queueMicrotask(() => inputEl?.focus());
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
        if (!live && voiceOpen) {
          // Every idle label offers the same way back, because they are all
          // the same situation to the person looking at them: the call is
          // gone and the logo is how you get it again.
          vState = { name: 'idle', label: 'line dropped — tap the logo to reconnect' };
        }
      },
      onBotTurnEnd: () => {},
    });
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
    <!-- **A fresh context is one tap from where you are.** It lived inside
         the drawer, which made starting over a two-step navigation *and* a
         naming decision; the surface people reach for most often was the
         one behind the most doors. It stays here on every width — the
         docked panel on a wide window has its own, and both call the same
         thing. -->
    <button class="newbtn header" onclick={newSession} title="new conversation" aria-label="new conversation">
      <svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round"><path d="M12 5v14M5 12h14" /></svg>
    </button>
    <span class="title" title={key}>{heading}</span>
    <div class="meta">
      <!-- §6.2's readout on the typed surface. The voice logo's tint only
           renders inside the voice overlay, so a typed run that earned a
           label used to broadcast an affect event nothing displayed. Same
           contract as the TUI badge: the wire word, shown only when a run
           earned one (null — neutral — is the overwhelming common case and
           shows nothing), cleared by the next clean run. -->
      {#if affect || valence}
        <span
          class="chip affect"
          title={`how the last run went, by mecha's own appraisal of its record — clears on the next clean run${valence ? ` · ${valence.positives} positive, ${valence.negatives} negative signal(s)${valence.partial ? ', partial: some evidence was unavailable' : ''}` : ''}`}
        >
          {#if affect}{affect}{/if}
          {#if valence}
            <!-- A two-sided bar, negative to the left of a centre tick and
                 positive to the right, so a run that went both ways shows
                 both rather than netting them — the same rule `Valence`
                 keeps in the record. Outline and hairline only: brand.md's
                 "hazard amber never fills an area" applies to the negative
                 side, which is drawn as a line. -->
            <span class="valence" aria-label={`negative ${valence.negative.toFixed(1)}, positive ${valence.positive.toFixed(1)}`}>
              <span class="neg" style:width={barWidth(valence.negative)}></span>
              <span class="tick"></span>
              <span class="pos" style:width={barWidth(valence.positive)}></span>
            </span>
            {#if valence.partial}<span class="partial">…</span>{/if}
          {/if}
        </span>
      {/if}
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

  {#if drawer || docked}
    <!-- Docked, the panel is the same markup with the modal parts left
         off: no scrim to dismiss, no slide-in, and nothing to close —
         a sidebar that is always there is not a thing you opened. -->
    {#if !docked}
      <div class="scrim" onclick={() => (drawer = false)} aria-hidden="true"></div>
    {/if}
    <aside class="drawer" class:docked>
      <div class="drawer-head">
        <span class="drawer-title">Sessions</span>
        <button class="newbtn" onclick={newSession}>
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
            {#if s.title?.startsWith('task: ')}<span class="dkind">task</span>{/if}
            {#if s.taint?.untrusted}<span class="railtaint">▲</span>{/if}
          </button>
        {/each}
        <div class="dsection">earlier</div>
        {#if history === null}
          <div class="dempty">reading the record…</div>
        {:else}
          {#each history.filter((h) => !h.attached_key) as h}
            <button class="drow past" onclick={() => resumeSession(h.id)}>
              <!-- The name it earned, and the opening line for one that has
                   not earned one yet (or was renamed past where the listing
                   scan reads). -->
              <span class="dsnippet">{nameOf(h.title) || h.snippet}</span>
              <span class="dmeta">
                {#if h.kind === 'voice'}<span class="dkind">voice</span>{/if}
                {#if h.kind === 'task'}<span class="dkind">task</span>{/if}
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

  {#if task}
    <!-- **The goal, above the conversation about it.** A task chat that
         only stated its subject in the opening turn made the subject scroll
         away — and "I can't see what the goal is" was a complaint about a
         page that knew and did not say. Current state belongs above the
         transcript, on the todo panel's own reasoning. -->
    <div class="taskhead">
      <div class="taskname">{task.name}</div>
      <div class="taskmeta">
        {#if task.project}<span class="tchip">{task.project}</span>{/if}
        {#if task.context}<span class="tchip">{task.context}</span>{/if}
        {#if task.due_at}<span class="tchip" class:tover={task.overdue}>due {task.due_at}</span>{/if}
        {#if task.defer_until}<span class="tchip">deferred to {task.defer_until}</span>{/if}
        {#if task.captured_from?.kind}
          <!-- The pointer, never the prose: kind and where, and not the
               subject line, which is somebody else's words. Reading it is
               the board's affordance, one tap away there. -->
          <span class="tchip">from {task.captured_from.kind}</span>
        {/if}
      </div>
      <div class="taskacts">
        <button class="handbtn" disabled={handing || running} onclick={handOver}>
          {running ? 'working — hand over when it pauses' : 'let it carry on without me'}
        </button>
      </div>
      {#if handNote}<div class="handnote">{handNote}</div>{/if}
    </div>
  {/if}
  {#if todo.length}
    <!-- Above the transcript rather than in it: the plan is current state,
         not something that was said at a moment. Scrolling back through a
         long run should not scroll past where it got to. -->
    <div class="todo">
      <button class="todohead" onclick={() => (todoOpen = !todoOpen)}>
        <span class="todocount">{todo.filter((i) => i.status === 'completed').length}/{todo.length}</span>
        <span class="todonow">
          {todo.find((i) => i.status === 'in_progress')?.content ?? 'plan'}
        </span>
        <span class="todochev">{todoOpen ? '−' : '+'}</span>
      </button>
      {#if todoOpen}
        <ul class="todolist">
          {#each todo as item}
            <li class:tdone={item.status === 'completed'} class:tnow={item.status === 'in_progress'}>
              <span class="tmark">{MARK[item.status] ?? '[ ]'}</span>{item.content}
            </li>
          {/each}
        </ul>
      {/if}
    </div>
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
        <!-- The chip names the call and says which one it was; the tap opens
             the whole of it — what it was called with, then what came back,
             both capped server-side. The chevron is the affordance, so it
             turns: a disclosure arrow that never moves is what made this row
             look inert. Rendered as TEXT only (Svelte escapes interpolation):
             results carry third-party content and an MCP call's arguments can
             echo it, so this page displays them, never interprets them. -->
        {@const digest = toolDigest(entry.draft)}
        {@const detail = !!(entry.draft || entry.args || entry.preview)}
        <div class="tool" class:err={entry.is_error} class:blocked={entry.blocked}>
          <button
            class="toolhead"
            disabled={!detail}
            aria-expanded={detail ? entry.open === true : undefined}
            onclick={() => (entry.open = !entry.open)}
          >
            <svg class="toolchev" class:down={entry.open} viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 6l6 6-6 6" /></svg>
            <span class="toolname">{entry.name}</span>
            {#if digest}<span class="tooldigest">{digest}</span>{/if}
          </button>
          {#if entry.pending}<span class="tool-state">running…</span>
          {:else if entry.blocked}<span class="tool-state">blocked</span>
          {:else if entry.is_error}<span class="tool-state">failed</span>{/if}
        </div>
        {#if entry.open}
          <div class="toolpanel">
            {#if entry.draft}
              {#each entry.draft.headers as [k, v]}
                <div class="tfield"><span class="tkey">{k.replace(/_/g, ' ')}</span><span>{v}</span></div>
              {/each}
              {#if entry.draft.body}<pre class="tbody">{entry.draft.body}</pre>{/if}
              <!-- After the body and never behind the toggle, for the reason
                   the approval card gives: `shell` has no header or body
                   field at all, so hiding `other` renders an empty panel
                   over `rm -rf build`. -->
              {#each entry.draft.other as [k, v]}
                <div class="tfield"><span class="tkey">{k.replace(/_/g, ' ')}</span><span>{v}</span></div>
              {/each}
              {#if entry.args}
                <button class="qmore" onclick={() => (entry.rawOpen = !entry.rawOpen)}>
                  {entry.rawOpen ? 'less' : 'the whole call'}
                </button>
                {#if entry.rawOpen}<pre class="toolout">{entry.args}</pre>{/if}
              {/if}
            {:else if entry.args}
              <pre class="toolout">{entry.args}</pre>
            {/if}
            <!-- "still running", "answered nothing" and "answered this" are
                 three different readings, and an absent block would collapse
                 the first two into the third. -->
            <!-- A refusal is not an answer. The reload path cannot tell the
                 two apart — it sees only `is_error` on the recorded result —
                 so the live view is the more precise of the two here, not a
                 second opinion about the same fact. -->
            {#if entry.blocked && entry.preview}
              <div class="tsep">refused with</div>
              <pre class="toolout">{entry.preview}</pre>
            {:else if entry.preview}
              <div class="tsep">{entry.is_error ? 'failed with' : 'answered'}</div>
              <pre class="toolout">{entry.preview}</pre>
            {:else if entry.pending}
              <div class="tsep">still running</div>
            {:else if !entry.blocked}
              <div class="tsep">answered with nothing</div>
            {/if}
          </div>
        {/if}
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
        bind:this={inputEl}
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
          class:notable={affect || (valence && valence.negatives > 0)}
          disabled={vState.name !== 'idle'}
          onclick={reconnectVoice}
          aria-label={vState.name === 'idle' ? 'reconnect the call' : `mecha ${vState.label}`}
        >
          <svg viewBox="0 0 63 54" width="112" height="96" aria-hidden="true">
            <!-- §6.2's readout. `affect` is `null` on the overwhelming
                 common (neutral) case, which is what leaves the mark alone
                 — never a word, never a fill change. brand.md: "hazard
                 amber never fills an area — lines, ticks and single
                 characters only," so the tint is a thin outline on the
                 button (see `.logo.notable` below), not the mark's own
                 solid fill, which an earlier version of this got wrong. -->
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
      <!-- Voice and rate moved to the settings page (the gear on Home):
           they are preferences, not call controls, and a pane that is
           mostly a form is a worse call surface. voice-core still applies
           the remembered choice the moment the data channel opens. -->
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
    /* What the docked sessions panel is positioned against on a wide
       window — on a phone the panel is `fixed` and this does nothing. */
    position: relative;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    /* The name of the conversation now earns its place here — it used to be
       the key, which was `main` or nothing. On a phone the three status
       chips and a title do not fit on one line, and the title is the half
       that would silently shrink to zero (it is the only flexible item), so
       the chips wrap under it instead of squeezing it out. */
    flex-wrap: wrap;
    row-gap: 6px;
    padding: 22px var(--gutter-gear) 6px var(--gutter);
  }
  .title {
    flex: 1 1 auto;
    min-width: 0;
    margin-left: 2px;
    font-weight: 500;
    font-size: 17px;
    letter-spacing: -0.02em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    flex-wrap: wrap;
    margin-left: auto;
    gap: 6px;
  }
  .chip.taint {
    color: var(--hazard);
  }
  /* Deliberately NOT hazard amber — found on review: this chip sits in the
     same row as the taint chip, and two amber chips side by side make "this
     conversation holds untrusted content" (a security posture) and "the
     last run went badly" (a mood) read as the same class of signal.
     brand.md scopes amber to held sends, read-only, and the called-out
     rule; the appraisal readout is none of those, so it takes the muted
     outline instead. Outline-only either way — no fills. */
  .chip.affect {
    color: var(--text-muted);
    border: 1px solid var(--text-muted);
    background: none;
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .chip.affect .valence {
    display: inline-grid;
    grid-template-columns: 24px 1px 24px;
    align-items: center;
    height: 8px;
  }
  .chip.affect .valence .neg {
    justify-self: end;
    height: 2px;
    background: var(--hazard);
  }
  .chip.affect .valence .pos {
    justify-self: start;
    height: 2px;
    background: var(--text-muted);
  }
  .chip.affect .valence .tick {
    width: 1px;
    height: 8px;
    background: var(--text-muted);
  }
  .chip.affect .partial {
    color: var(--text-muted);
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
  .newbtn.header {
    padding: 0;
    width: 34px;
    min-height: 34px;
    justify-content: center;
    color: var(--accent-400);
    flex-shrink: 0;
  }
  .newbtn.header:hover {
    color: var(--accent-300);
    border-color: var(--accent-500);
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
  .taskhead {
    padding: 0.6rem 0.8rem;
    border-bottom: 1px solid var(--line, #2a2a38);
    background: var(--bg2, #14141c);
  }
  .taskname {
    font-weight: 600;
    font-size: 0.95rem;
    line-height: 1.3;
  }
  .taskmeta {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    margin-top: 0.4rem;
  }
  .tchip {
    font-size: 0.74rem;
    padding: 0.1rem 0.4rem;
    border: 1px solid var(--line, #2a2a38);
    border-radius: 5px;
    opacity: 0.85;
  }
  .tover {
    color: var(--warn, #e0a458);
    border-color: currentColor;
  }
  .taskacts {
    margin-top: 0.5rem;
  }
  .handbtn {
    font: inherit;
    font-size: 0.8rem;
    padding: 0.3rem 0.6rem;
    border-radius: 6px;
    border: 1px solid var(--line, #2a2a38);
    background: transparent;
    color: inherit;
  }
  .handbtn:disabled {
    opacity: 0.5;
  }
  .handnote {
    margin-top: 0.35rem;
    font-size: 0.76rem;
    opacity: 0.75;
  }
  .todo { border-bottom: 1px solid var(--accent-900); background: var(--surface); flex: 0 0 auto; }
  .todohead { display: flex; align-items: center; gap: 8px; width: 100%; background: none; border: none; padding: 8px 14px; cursor: pointer; text-align: left; }
  .todocount { font-family: var(--mono); font-size: 11px; color: var(--accent-400); }
  .todonow { flex: 1; font-size: 12px; color: var(--text-muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .todochev { font-family: var(--mono); font-size: 13px; color: var(--text-muted); }
  .todolist { list-style: none; margin: 0; padding: 0 14px 10px; }
  .todolist li { font-size: 12px; line-height: 1.6; color: var(--text); }
  .tmark { font-family: var(--mono); color: var(--text-muted); margin-right: 7px; }
  /* Done is dimmed rather than struck through: a finished step is still part
     of the record of what happened, and a page of strikethrough reads as a
     list of mistakes. */
  .todolist li.tdone { color: var(--text-muted); }
  .todolist li.tnow { color: var(--accent-400); }
  .transcript {
    flex: 1;
    overflow-y: auto;
    padding: 16px var(--gutter);
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  /* A column flex container hands out *negative* free space too, and a
     child that is itself a scroll container has an automatic minimum size of
     zero rather than a content-sized one — so it is the one kind of child
     this column can crush. `.toolout` used to sit here directly: measured at
     700x400 on the shape this file had before, a 37px result rendered 28px
     high, which is a line of output cut through the middle on exactly the
     transcripts long enough to want reading.

     It is nested a level down now, under a panel with visible overflow, so
     the squeeze has no way in — but the next `pre` or scroll box someone
     drops straight into the transcript would land right back on it, and it
     would look like a rendering glitch rather than a layout rule.
     `.drawer-scroll` carries the same line for the same reason. */
  .transcript > * {
    flex-shrink: 0;
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
  .toolhead {
    display: flex;
    align-items: center;
    gap: 7px;
    flex: 1;
    min-width: 0;
    background: none;
    border: none;
    padding: 2px 0;
    min-height: 28px;
    font: inherit;
    color: inherit;
    text-align: left;
    cursor: pointer;
  }
  .toolhead:disabled {
    cursor: default;
  }
  .toolchev {
    color: var(--accent-700);
    flex-shrink: 0;
    transition: transform 120ms ease;
  }
  .toolchev.down {
    transform: rotate(90deg);
  }
  .toolname {
    flex-shrink: 0;
  }
  /* Which call this was, on the closed chip. Truncated rather than wrapped:
     the chip is one line, and a long path is recognised by its end as much
     as its start — so the box scrolls it under the ellipsis rather than
     growing. */
  .tooldigest {
    color: var(--accent-700);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .toolpanel {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: -6px 0 0 18px;
  }
  .tfield {
    display: flex;
    gap: 10px;
    font-size: 13px;
    overflow-wrap: anywhere;
  }
  .tkey {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--text-muted);
    flex: 0 0 84px;
    padding-top: 3px;
  }
  .tbody {
    margin: 0;
    font-family: var(--mono);
    font-size: 11px;
    line-height: 1.5;
    color: var(--text);
    background: var(--void);
    border: 1px solid var(--accent-900);
    border-radius: var(--radius);
    padding: 10px 12px;
    max-height: 40vh;
    overflow: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .tsep {
    font-family: var(--mono);
    font-size: 10px;
    color: var(--text-muted);
  }
  .toolout { font-family: var(--mono); font-size: 11px; color: var(--text-muted); line-height: 1.5; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); padding: 10px 12px; margin: 0; max-height: 40vh; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; }
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
  /* §6.2's readout. A line, never a fill — brand.md's own rule for hazard
     amber, and the reason this is an outline around the mark rather than
     the mark's own colour. `outline-offset` keeps it a ring around the
     button, not touching the SVG's paths at all. */
  .logo.notable {
    outline: 2px solid var(--hazard);
    outline-offset: 4px;
    border-radius: var(--radius);
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

  /* ---- the wide window ----
     Two steps, because two different things stop fitting at two different
     widths. At 900px the shell has already moved the nav to a left rail
     (App.svelte), so the header's reserved gear corner comes back and the
     transcript stops stretching: it keeps `--measure` and pads the rest
     away, which leaves the scrollbar at the window edge where a desktop
     expects it rather than floating mid-page. At 1180px there is room for
     the session list to simply stay open — the one thing a phone could
     not afford, and the reason the drawer existed. */
  @media (min-width: 900px) {
    /* The composer and the two state panels take the transcript's own
       measure — they are the same column, and only the transcript gets it
       from the shared gutter (the others are floored lower on a phone,
       where 20px of chrome is 5% of the screen). */
    footer,
    .taskhead,
    .todo {
      padding-inline: var(--gutter);
    }
    /* An 82%-wide bubble is a phone measure; against an 880px column it is
       a very long line to read back. */
    .bubble {
      max-width: 66%;
    }
    .answer {
      max-width: 100%;
    }
  }
  @media (min-width: 1180px) {
    .chat {
      padding-left: var(--sessions);
    }
    .drawer.docked {
      position: absolute;
      width: var(--sessions);
      border-right: 1px solid var(--accent-900);
      padding-top: 0;
      animation: none;
    }
    /* The one control the docked panel makes redundant. */
    .menubtn {
      display: none;
    }
    /* A call takes over the conversation, not the list of them. */
    .voice-overlay {
      left: var(--sessions);
    }
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
