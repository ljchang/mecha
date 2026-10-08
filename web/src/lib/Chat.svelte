<script>
  import { tick } from 'svelte';
  import { apiFetch as fetch } from './api.js';
  import { tameName, validName } from './library.js';
  import ModelChip from './ModelChip.svelte';
  import ChatProse from './ChatProse.svelte';
  import PictureQueue from './PictureQueue.svelte';
  import { replyContext } from './speech.js';
  import EditModal from './EditModal.svelte';
  import { composeEditMessage, maskName } from './image-edit.js';
  import { pictureOf, stillOut, waitingPictures, repeatedPictures, downloadPicture } from './picture.js';
  import { carriesFiles, droppedFiles, withAttachments } from './attach.js';
  import { rowSummary, ROUTING_KEYS } from './outbox-view.js';
  import { features } from './features.svelte.js';
  import { isShown } from './features.js';
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
  import { createVoiceSession, dropRing, readVoicePrefs } from '../../../scripts/voice/voice-core.js';
  import { historyLines, pendingSpeech } from './call-lines.js';

  let key = $state('main');
  let mode = $state('read_only');
  let rail = $state([]);
  let entries = $state([]);
  let streaming = $state('');
  let running = $state(false);
  // The chat's background jobs as the server last said (`queue` events and
  // the transcript's `queue`), for the queue panel.
  let queue = $state([]);
  // Counts `queue` events, so a transcript read that left before one arrived
  // does not lay its older line over it: events fire only on a change, and
  // nothing would put the newer line back (review of #607).
  let queueSeq = 0;
  // The agent's plan for this session, live. Rendered rather than summarised:
  // a paraphrase of a plan is a different plan, and this is the one part of a
  // long run that says how far it got rather than what is true.
  let todo = $state([]);
  // The board task this conversation is about, and whether a hand-over is in
  // flight. A task chat is the same chat with a subject.
  let task = $state(null);
  let handing = $state(false);
  // **Incognito** (`docs/INCOGNITO-DESIGN.md`): a chat nothing keeps. The
  // server says which kind a chat is (`incognito` on the transcript read);
  // the page adds the banner, End and the search notice, and its voice call
  // says it keeps nothing. `gone` is why an incognito chat is over — ended here, or
  // closed by the server while the page was away — and it replaces the
  // conversation, which the page forgets as well.
  let incognito = $state(false);
  let gone = $state(null);
  // The server's prefix (`incognito::KEY_PREFIX`), which its ordinary door
  // refuses: a key carrying it is an incognito chat before any read says so.
  const INCOGNITO_PREFIX = 'incognito-';
  // Why a new incognito chat from the gone screen was refused: that screen
  // draws no transcript, so a notice pushed there would go unseen.
  let goneNote = $state(null);
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

  // A refusal that also produced a result.
  //
  // The planning-phase path is the one denial that emits both, and the two
  // strings are not the same string. `ToolDenied` carries the label —
  // literally `"planning phase"` — while `ToolResult` carries the sentence
  // the model was actually handed: "`fs_write` is not available while
  // planning. Work out what to do and say so; leave the phase to carry it
  // out." The label arrives first and closes the row; the sentence is the
  // one worth reading, and it is what the row showed before one chip per
  // call was the rule.
  //
  // **By id only.** This is the one place a result may touch an
  // already-closed row, so it has to be the row that call opened and no
  // other — matching any looser would put one call's refusal onto another
  // call's row, which is the swap the id exists to stop.
  function fillRefusal(entries, ev) {
    if (ev.id == null || !ev.preview) return false;
    const row = entries.findLast((e) => e.kind === 'tool' && e.blocked && e.id === ev.id);
    if (!row) return false;
    row.preview = ev.preview;
    return true;
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

  // Which read of the plan is the latest. Two writers set it — this read and
  // the transcript read, which carries the plan too — and the transcript
  // read is the slow one mid-run (it carries the history), so its answer can
  // land after a fresher plan read and put back a plan the run has revised.
  let todoGen = 0;

  async function refreshTodo() {
    // The chat it was asked for: an answer landing after a switch is that
    // chat's plan, not this one's.
    const sessionKey = key;
    const gen = ++todoGen;
    try {
      const res = await fetch(`/api/chat/${sessionKey}/todo`);
      if (!res.ok) return;
      const plan = (await res.json()).todo ?? [];
      if (sessionKey === key && gen === todoGen) todo = plan;
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

  // **A read taken mid-run is the history, not the run.** While a run holds
  // the conversation the server answers with what the run started from; the
  // run itself reaches this page only as the events it streams after the
  // page subscribed. A page opened mid-run — or a phone whose stream died
  // in the background and reconnected, which is most runs long enough to
  // lock the screen over (an image edit) — has missed the stretch between,
  // and that stretch is where a picture's `image_generate` result lives.
  // So `done` re-reads when the last read found a run in flight.
  let partialRun = false;
  // How many `done`s this page has taken. The stream is opened before the
  // read, so a `done` can overtake a read that then reports the run in
  // flight; the read compares against this to know it is already stale.
  let doneSeq = 0;
  // Where this page's own additions start — what the transcript does not
  // hold, which a re-read must carry over rather than drop.
  let liveFrom = 0;

  // What only this page holds, and a re-read must carry over: a draft card,
  // a notice, and words the run never took — never folded into the
  // conversation, so they sit beside the notice saying send again. A
  // delivered message is in the transcript; carrying it would draw it twice.
  const pageOnly = (e) =>
    e.kind === 'draft' ||
    e.kind === 'notice' ||
    (e.kind === 'user' && !!e.queued && e.delivery !== 'delivered');

  // The chat on screen's signal, aborted by the stream effect's cleanup when
  // the page switches away or unmounts, so a catch-up still on the wire
  // writes nothing into a view that has moved on.
  let viewSignal = null;
  // Which read is the latest. Two can be on the wire at once — the stream's
  // own and a catch-up, or two catch-ups — and an older answer landing last
  // would put back the run the newer one saw end.
  let loadGen = 0;

  // A re-read that keeps what only this page holds. The cards are taken in
  // `load`, in the same step that replaces the list, never before its
  // `await`: a card pushed while the read is on the wire is in the list by
  // then, and a read that fails, is superseded, or finds the chat gone
  // replaces nothing and so carries nothing — no duplicate, and nothing put
  // back into an incognito chat that `forget` has just emptied.
  function catchUp(sessionKey) {
    return load(sessionKey, viewSignal, { carry: true });
  }

  // True when it replaced the transcript with the server's.
  async function load(sessionKey = key, signal, { carry = false } = {}) {
    const seq = doneSeq;
    const qSeq = queueSeq;
    const gen = ++loadGen;
    const planGen = ++todoGen;
    try {
      const res = await fetch(`/api/chat/${sessionKey}`, { signal });
      // Reaped between the open and this read: the gone screen, not an
      // error strip (review of #326).
      if (res.status === 410) {
        if (!signal?.aborted && sessionKey === key) closeIncognito('closed');
        return false;
      }
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${(await res.text()).trim()}`);
      const data = await res.json();
      if (signal?.aborted || sessionKey !== key || gen !== loadGen) return false;
      const carried = carry ? entries.slice(liveFrom).filter(pageOnly) : [];
      entries = data.entries.map((e) =>
        e.kind === 'tool' ? { ...e, pending: false } : e
      );
      running = data.running;
      if (queueSeq === qSeq) queue = data.queue ?? [];
      partialRun = !!data.held_by_run;
      // Before the carried cards, so they stay page-only for the next re-read.
      liveFrom = entries.length;
      entries.push(...carried);
      // A `done` landed while this read was on the wire, and the read still
      // saw the run: that run is over and nothing will say so again.
      if (partialRun && doneSeq !== seq) {
        partialRun = false;
        queueMicrotask(() => catchUp(sessionKey));
      }
      // What this conversation is about, when it is about a board task.
      // Absent for an ordinary chat, which renders exactly as before.
      task = data.task ?? null;
      incognito = data.incognito ?? false;
      if (planGen === todoGen) todo = data.todo ?? [];
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
      return true;
    } catch (e) {
      if (!signal?.aborted && sessionKey === key) error = String(e?.message ?? e);
    }
    return false;
  }

  // A POST can resolve before or after its broadcast. Correlate by request,
  // never by text: two devices may deliberately send identical words.
  const receivedInputs = new Set();
  const inputDelivery = new Map();
  function markDelivery(ids, delivery) {
    const changed = new Set(ids);
    for (const id of ids) inputDelivery.set(id, delivery);
    entries = entries.map(e => changed.has(e.request_id) ? { ...e, delivery } : e);
  }
  function receiveInput(ev) {
    if (ev.request_id && receivedInputs.has(ev.request_id)) return;
    if (ev.request_id) receivedInputs.add(ev.request_id);
    pushEntry({ kind: 'user', text: ev.text, request_id: ev.request_id, queued: ev.type === 'queued', spoken: ev.spoken, delivery: inputDelivery.get(ev.request_id) ?? 'queued' });
    if (ev.type === 'user') running = true;
  }

  function subscribe(sessionKey) {
    // tailscale serve injects the identity header on this request too —
    // EventSource cannot set headers, and never needs to here.
    const source = new EventSource(`/api/chat/${sessionKey}/events`);
    source.onmessage = (raw) => {
      // An incognito chat that has gone keeps nothing in this tab, and a run
      // cancelled by End still streams its partial turn: dropped, not drawn
      // (review of #326). The effect below closes the stream as well.
      if (gone) return;
      const ev = JSON.parse(raw.data);
      switch (ev.type) {
        case 'delta':
          streaming += ev.text;
          scrollDown();
          break;
        case 'queued_delivered':
          markDelivery([ev.request_id], 'delivered');
          break;
        case 'queue':
          queueSeq += 1;
          queue = ev.jobs ?? [];
          break;
        case 'queued_discarded':
          markDelivery(ev.request_ids, 'discarded');
          break;
        case 'queued':
        case 'user':
          receiveInput(ev);
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
          // A picture finished after its turn (§5.4): its row closed when the
          // call answered "being made", and the finished result lands on it
          // by id — never on a row that already holds its answer.
          const late = open
            ? null
            : entries.find((e) => e.kind === 'tool' && e.id != null && e.id === ev.id && stillOut(e));
          if (!open && !late) fillRefusal(entries, ev);
          const row = open ?? late;
          if (row) {
            row.pending = false;
            row.is_error = ev.is_error;
            row.preview = ev.preview;
          }
          // Every plan change already arrives here as a tool call, so the
          // list needs no event of its own — re-read on the one that means
          // it changed. The plan's own read, not the transcript's: a
          // mid-run transcript read carries the whole history, and a
          // model revising its plan often would pay for it every time.
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
          doneSeq += 1;
          flushStreaming();
          running = false;
          taint = { private: ev.taint_private, untrusted: ev.taint_untrusted };
          // A run that produced no `affect` event was `Neutral` — the
          // server never says so out loud, so silence is read as such here.
          // Reset here rather than at run start: `WireEvent::Affect` is
          // always sent before `Done` within one `begin_turn`, so by the
          // time this fires the flag has already done its job for this
          // run. Historically only spoken turns broadcast their start,
          // so resetting at `Done` also covered observers of typed runs.
          // It still covers an observer that joins after the start event.
          if (!sawAffectThisRun) {
            affect = null;
            valence = null;
          }
          sawAffectThisRun = false;
          if (!ev.ok && ev.error) pushEntry({ kind: 'notice', text: ev.error });
          // The backstop for a call whose result never arrived: a
          // cancelled run, a dropped stream, a turn that ended mid-flight.
          // Marked rather than merely closed — "nothing came back" and "we
          // stopped listening" are different findings, and closing the row
          // silently would file the second under the first. Before this row
          // carried the call that distinction had nowhere to show; now the
          // row opens, and it must not answer for a call that never did.
          entries = entries.map((e) =>
            e.kind === 'tool' && e.pending ? { ...e, pending: false, unfinished: true } : e
          );
          // The conversation is back in the server's hands by now (it is
          // handed back before `done` is sent), so this read is the whole of it.
          if (partialRun) {
            partialRun = false;
            catchUp(sessionKey);
          }
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
      // The belt under `done`: one the page never received — dropped with a
      // lagged batch, or sent before this page's stream subscribed — leaves
      // `partialRun` set with nothing left to clear it. The rail already
      // says whether each chat's run is live, every 20 seconds, so a run it
      // calls over is caught up here. Only on a row that says so: a chat
      // missing from the rail is not evidence the run ended.
      const sessionKey = key;
      if (partialRun && rail?.find((s) => s.key === sessionKey)?.running === false) {
        partialRun = false;
        catchUp(sessionKey);
      }
    } catch {
      // the rail is a convenience; the transcript is the truth
    }
  }

  function switchTo(k) {
    if (k === key) return;
    // A call is bound to the chat it was opened in (`startVoice`), so a
    // switch that crosses the incognito line ends it: into one, a recorded
    // call must not go on under a page that says nothing is kept; out of
    // one, what was said there must not stay on screen (review of #376).
    // The uplink ring is audio of the chat being left and outlives calls on
    // purpose; an incognito chat's must not outlive the visit (the composer
    // below is the same rule).
    if (incognito || k.startsWith(INCOGNITO_PREFIX)) {
      endVoice();
      vEntries = [];
      vTyped = ''; // a line typed into the call is the composer's kind (review of #499)
    }
    if (incognito) dropRing(key);
    key = k;
    receivedInputs.clear();
    inputDelivery.clear();
    entries = [];
    // Or the last chat's pictures stay in line under this one for a round
    // trip (the #418 pattern; review of #607).
    queue = [];
    streaming = '';
    usage = null;
    taint = null;
    // What the catch-up knew of the last chat. The new key's first read sets
    // both again; reset here so nothing in between (the rail's belt) acts on
    // the old chat's run. `doneSeq` is a counter compared by a read against
    // its own start, and a read for the old key is dropped by its key check.
    partialRun = false;
    liveFrom = 0;
    // Same rule as everywhere else this readout guards against staleness
    // (the TUI's `/clear`, voice's `Hosted::Unknown` fall-through): the
    // tint describes the *previous* conversation's last run, and nothing
    // else here would clear it — `/api/chat/{key}` carries no affect
    // field, and the new key's own SSE subscription emits `Affect` only
    // once a run there finishes.
    affect = null;
    valence = null;
    sawAffectThisRun = false;
    // Leaving an incognito chat by any door but End: what was typed or
    // attached there must not follow you into a chat that is recorded, where
    // pressing send would write it into a transcript (review of #326). An
    // ordinary chat's unsent text still travels, as it always has.
    if (incognito) {
      draft = '';
      attachments = [];
      todo = [];
    }
    // Unlike the draft, the edit modal is never portable: it holds a path in
    // the chat it was opened in, and sending it from here would upload the
    // mask into this chat's jail and name the other chat's picture in this
    // transcript (review of #429).
    editing = null;
    pictureNote = null; // a path in the chat being left, as the modal's is
    incognito = false;
    gone = null;
    goneNote = null;
  }

  // What the page itself holds of a conversation. An incognito chat that has
  // ended must not live on in this tab's memory either.
  function forget() {
    // A call still speaking into the chat that has gone ends with it, and
    // what it showed and buffered goes too.
    endVoice();
    vEntries = [];
    vTyped = '';
    dropRing(key);
    entries = [];
    streaming = '';
    draft = '';
    attachments = [];
    editing = null; // the modal carries this chat's draft and paths too
    pictureNote = null; // and a download's note names one of its pictures
    todo = [];
    usage = null;
    taint = null;
    affect = null;
    valence = null;
  }

  function closeIncognito(why) {
    if (gone) return;
    gone = why;
    running = false;
    forget();
    loadRail();
  }

  /// **A chat nothing keeps**, through its own door: never a flag on the
  /// ordinary open. A refusal (no local model, no RAM-backed room, a hook
  /// that could not be honoured) says why, in the chat you were in.
  async function newIncognito() {
    drawer = false;
    try {
      const res = await fetch('/api/incognito', { method: 'POST' });
      if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      const data = await res.json();
      switchTo(data.key);
      incognito = true;
      // Another chat's attachments are paths in another jail; they would not
      // resolve in the room.
      attachments = [];
      // `switchTo` returns early on the same key, and it is what clears this.
      gone = null;
      // From the gone screen the composer is not mounted yet — it lives in
      // `{#if gone}`'s other arm — so wait for the flush that draws it.
      await tick();
      inputEl?.focus();
    } catch (e) {
      const why = `incognito is unavailable: ${e?.message ?? e}`;
      if (gone) goneNote = why;
      else pushEntry({ kind: 'notice', text: why });
    }
  }

  async function endIncognito() {
    if (!incognito || gone) return;
    try {
      const res = await fetch(`/api/incognito/${key}/end`, { method: 'POST' });
      // 410 is "already closed" — the promise is kept either way.
      if (!res.ok && res.status !== 410) {
        throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      }
      closeIncognito('ended');
    } catch (e) {
      pushEntry({ kind: 'notice', text: `end failed: ${e?.message ?? e}` });
    }
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

  // **Archive files a conversation away; delete forgets it.** Archived, the
  // record stays whole — learning, appraisal and the graph still read it —
  // and only this list stops showing it; deleted, the transcript and
  // everything derived from it are gone (`serve/archive.rs`). Both act on
  // the transcript's id: an earlier conversation has no key, and an open
  // one is let go of server-side, so it leaves the rail too.
  let menuFor = $state(null);
  let archivedOpen = $state(false);
  let archivedRows = $state(null);
  /// What the last archive or delete did, said in the drawer rather than in
  /// a conversation — the conversation it was about may be the one that
  /// just went. `retry` is the id of a delete that did not finish.
  let drawerNote = $state(null);

  async function loadArchived() {
    try {
      const res = await fetch('/api/history?archived=true');
      if (res.ok) archivedRows = (await res.json()).sessions;
    } catch {
      // the drawer is a convenience; the transcript is the truth
    }
  }

  function toggleArchived() {
    archivedOpen = !archivedOpen;
    if (archivedOpen) loadArchived();
  }

  function refreshLists() {
    loadRail();
    loadHistory();
    if (archivedOpen) loadArchived();
  }

  /// Showing the conversation that is going away: move to a fresh one, as
  /// the new-chat button would. Read before the lists refresh, while the
  /// rail still says which key held it.
  function leaveIfShowing(id) {
    if (rail.find((s) => s.id === id)?.key === key) {
      switchTo(`chat-${Math.random().toString(36).slice(2, 8)}`);
    }
  }

  async function archiveChat(id, archived = true) {
    menuFor = null;
    try {
      // Two literal calls rather than one built path: the docs demo's
      // check reads each `fetch` literal to prove every endpoint is answered.
      const res = archived
        ? await fetch(`/api/sessions/${encodeURIComponent(id)}/archive`, { method: 'POST' })
        : await fetch(`/api/sessions/${encodeURIComponent(id)}/unarchive`, { method: 'POST' });
      if (!res.ok) throw new Error((await res.text()).trim());
      if (archived) leaveIfShowing(id);
      drawerNote = { text: archived ? 'archived — find it under “archived” below' : 'restored to the list' };
    } catch (e) {
      drawerNote = { text: `${archived ? 'archive' : 'restore'} failed: ${e?.message ?? e}` };
    }
    refreshLists();
  }

  async function deleteChat(id, label) {
    menuFor = null;
    const ok = confirm(
      `Permanently delete “${label}”?\n\n` +
        'The conversation, its files, its staged drafts and questions, anything learned from it, ' +
        'and its memory in the knowledge graph are all removed. This cannot be undone.\n\n' +
        'To hide it but keep the record, archive it instead.',
    );
    if (!ok) return;
    try {
      const res = await fetch(`/api/sessions/${encodeURIComponent(id)}`, { method: 'DELETE' });
      if (!res.ok) throw new Error((await res.text()).trim());
      const report = await res.json();
      leaveIfShowing(id);
      drawerNote = report.complete
        ? { text: 'deleted', left: report.residue }
        : {
            text: `only partly deleted — ${report.errors.join('; ')}`,
            retry: { id, label },
          };
    } catch (e) {
      drawerNote = { text: `delete failed: ${e?.message ?? e}` };
    }
    refreshLists();
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
      // Opening an archived one un-archives it server-side; the lists catch up.
      refreshLists();
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
    if (!entries.length && !running && !incognito) {
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
    const sessionKey = key;
    // Read here so the stream closes when an incognito chat goes — its
    // cleanup below runs — and is not reopened for a chat that is over.
    if (gone) return;
    const controller = new AbortController();
    viewSignal = controller.signal;
    let source;
    let retry;
    let retryDelay = 1500;
    const reconnect = () => {
      source?.close();
      source = null;
      clearTimeout(retry);
      if (!controller.signal.aborted) {
        retry = setTimeout(connect, retryDelay);
        retryDelay = Math.min(retryDelay * 2, 30_000);
      }
    };
    async function connect() {
      try {
        // GET and EventSource only read existing chats. Opening one is an
        // explicit, guarded mutation, also repeated after a server restart.
        const res = await fetch(`/api/chat/${sessionKey}`, {
          method: 'POST', signal: controller.signal,
        });
        if (res.status === 410) {
          // An incognito chat the server closed while this page was away
          // (idle, or a restart): nothing to reconnect to, by design.
          if (!controller.signal.aborted) closeIncognito('closed');
          return;
        }
        if (!res.ok) {
          const message = `HTTP ${res.status}: ${(await res.text()).trim()}`;
          if (controller.signal.aborted) return;
          // Authentication, invalid keys and other permanent client errors
          // need intervention, not an endless POST loop from every tab.
          if (res.status >= 400 && res.status < 500 && ![408, 429].includes(res.status)) {
            error = message;
            return;
          }
          throw new Error(message);
        }
        if (controller.signal.aborted) return;
        source = subscribe(sessionKey);
        source.onopen = () => { retryDelay = 1500; };
        source.onerror = reconnect;
        await load(sessionKey, controller.signal);
        if (!controller.signal.aborted) loadRail();
      } catch (e) {
        if (controller.signal.aborted) return;
        error = String(e?.message ?? e);
        reconnect();
      }
    }
    connect();
    return () => {
      controller.abort();
      clearTimeout(retry);
      source?.close();
    };
  });
  // **An open page is use** (owner's ruling, 2026-09-25): while this page
  // shows an incognito chat it says so once a minute, so reading or
  // uploading is not reaped mid-use. A closed tab or a sleeping phone stops
  // the pings, and the chat closes 30 minutes later. The answer is also how
  // the page learns the chat has gone.
  $effect(() => {
    if (!incognito || gone) return;
    const sessionKey = key;
    const ping = async () => {
      try {
        const res = await fetch(`/api/incognito/${sessionKey}/alive`, { method: 'POST' });
        if (res.status === 410 && sessionKey === key) closeIncognito('closed');
      } catch {
        // Offline for a moment; the next ping or the reconnect settles it.
      }
    };
    ping();
    const timer = setInterval(ping, 60_000);
    const onVisible = () => document.visibilityState === 'visible' && ping();
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      clearInterval(timer);
      document.removeEventListener('visibilitychange', onVisible);
    };
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
  // What the live call was opened against — read at connect time like its
  // key, never from the page's current chat, so the overlay describes where
  // the words are actually going.
  let vKey = $state(null);
  let vIncognito = $state(false);

  // The call shows this chat's own transcript, which spoken turns join over
  // the ordinary feed, the reply streaming in, and only the owner's speech
  // the transcript has not taken yet (the owner's ask, 2026-10-05: the call
  // shows the whole conversation, as the persona call does). One source, so
  // a re-read never draws a line twice.
  // Only while the page shows the conversation the call speaks into
  // (`vKey`, captured at connect): switched to another chat mid-call, the
  // pane falls back to the call's own lines rather than draw a different
  // conversation under the chip that names this one (review of #570).
  const vSame = $derived(voiceOpen && !!vKey && key === vKey);
  const vTranscript = $derived(vSame ? historyLines(entries) : []);
  const vReplying = $derived(vSame && streaming ? streaming.trim() : '');
  const vSpeaking = $derived(vSame ? pendingSpeech(vEntries, vTranscript) : vEntries);
  // Held at the bottom while the owner is there, left alone once they scroll
  // up to read: nothing arriving drags a reader back down.
  let vStick = true;
  function vScrolled() {
    if (voicePane) vStick = voicePane.scrollHeight - voicePane.scrollTop - voicePane.clientHeight < 40;
  }
  $effect(() => {
    void vTranscript.length, vReplying, vSpeaking.length, voicePane;
    if (vStick && voicePane) queueMicrotask(() => voicePane?.scrollTo({ top: voicePane.scrollHeight }));
  });

  function onTranscript({ who, text, interim }) {
    const last = vEntries.at(-1);
    if (last && last.who === who && last.interim) {
      last.text = text;
      last.interim = interim;
    } else {
      vEntries.push({ who, text, interim });
    }
    // A line growing in place changes no length the effect reads: follow it
    // here, while the owner is at the bottom (review of #570).
    if (vStick) queueMicrotask(() => voicePane?.scrollTo({ top: voicePane.scrollHeight }));
  }

  // `keep` is the reconnect path: the words already spoken stay on screen,
  // because the call dropping is not the conversation ending — D3 means the
  // session outlived the transport, and clearing the pane would say
  // otherwise to the one person who just watched it fail.
  function startVoice({ keep = false } = {}) {
    // connect() inside the tap handler — the audio unlock needs the gesture.
    if (!keep) vEntries = [];
    // A reconnect supersedes the notices that led to it: keep the speech.
    else vEntries = vEntries.filter((e) => e.who !== 'notice');
    vStick = true;
    // A line typed into another call — another chat, or an incognito one —
    // must not wait in this one's box (review of #499).
    if (!keep) {
      vTyped = '';
      vTyping = false;
    }
    vKey = key;
    vIncognito = incognito || key.startsWith(INCOGNITO_PREFIX);
    vState = { name: 'connecting', label: 'connecting' };
    vSession = createVoiceSession({
      // Same-origin: serve proxies to the loopback runner, so the offer
      // rides the owner guard and no cross-origin fetch exists to fail.
      offerUrl: '/api/offer',
      offerHeaders: { 'X-Mecha-Request': '1' },
      // D3: the call is this conversation. Read at connect time rather
      // than bound reactively — switching sessions mid-call must not
      // silently redirect the words being spoken into a different one.
      sessionKey: key,
      // An incognito call goes on only if the answer says nothing of it is
      // logged (`refusesAnswer`). Read off the key being sent, not only the
      // page's flag, which the transcript read sets a round trip after a
      // switch (review of #376).
      requireUnlogged: incognito || key.startsWith(INCOGNITO_PREFIX),
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
    // A fresh session's mic starts live; mute and typing carry over a
    // reconnect, so the track is set from them once it exists (review of
    // #499, pass 6).
    vSession
      .connect()
      .then(applyMic)
      .catch((e) => {
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

  // Listen's voice for the assistant's replies: both halves of what
  // Settings → Voice stores, as an assistant call applies them (review of
  // #502).
  function ownerVoice() {
    const p = readVoicePrefs();
    return { voice: p.voice ?? null, speed: p.speed ?? null };
  }

  let vMuted = $state(false);
  function toggleMute() {
    if (!vSession) return;
    vMuted = !vMuted;
    applyMic();
  }

  // Typing into the call (the owner's ask, 2026-10-01): a typed line is a
  // turn answered aloud, as a spoken one is. The mic is paused while the box
  // has focus — keys and a room are not words — and given back as it was;
  // the mute button stays the owner's. Same as a persona call
  // (`PersonaCall.svelte`).
  let vTyped = $state('');
  let vTyping = $state(false);
  function typingStart() {
    vTyping = true;
    applyMic();
  }
  function typingEnd() {
    vTyping = false;
    applyMic();
  }
  // The mic from both at once, wherever either changes: "mic paused while
  // you type" is a privacy claim, so it must not rest on the browser moving
  // focus off the box when mute is tapped (review of #499, pass 5).
  function applyMic() {
    vSession?.setMicEnabled(!vMuted && !vTyping);
  }
  function sendTyped() {
    if (vSession?.sendText(vTyped)) vTyped = '';
  }

  function endVoice() {
    try {
      vSession?.end();
    } finally {
      vSession = null;
      voiceOpen = false;
      vLevel = 0;
      // A hang-up is not a dropped line, so an incognito call's ring has no
      // reconnect to carry audio into; the reconnect path ends its session
      // directly and keeps the ring (review of #376).
      if (vIncognito) dropRing(vKey);
    }
  }

  $effect(() => () => vSession?.end());

  async function send() {
    // Named in the text and listed beside it (`withAttachments`).
    const attached = [...attachments];
    const text = withAttachments(draft.trim(), attached);
    attachments = [];
    if (!text) return;
    draft = '';
    const sessionKey = key;
    try {
      // Tailnet HTTP pages may not expose the secure-context UUID method.
      // This id correlates UI events; it is not an authorization token.
      const request_id = globalThis.crypto?.randomUUID?.()
        ?? `${Date.now()}-${Math.random().toString(36).slice(2)}`;
      const res = await fetch(`/api/chat/${sessionKey}/send`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ text, request_id, attachments: attached }),
      });
      if (res.status === 410) {
        if (sessionKey === key) closeIncognito('closed');
        return;
      }
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      if (sessionKey !== key) return;
      if (data.started || data.steered) {
        receiveInput({ type: data.started ? 'user' : 'queued', text, request_id, spoken: false });
      }
      // Pictures the text names that the model was not shown — said here,
      // since the chip is already gone. Only a blind model is a reason the
      // server can name; the rest (the cap, an unreadable file, a chat with
      // no workspace) are said as what happened, not guessed at.
      if (data.started && data.pictures_not_shown > 0) {
        pushEntry({
          kind: 'notice',
          text: data.model_sees
            ? `${data.pictures_not_shown} picture(s) went in by name only — the model was not shown them.`
            : 'This model cannot see images, so the picture(s) went in by name only.',
        });
      }
      // A steer carries text only, so a picture sent into a working run is
      // named and not shown.
      if (data.steered && attached.some((p) => /\.(png|jpe?g|gif|webp)$/i.test(p))) {
        pushEntry({
          kind: 'notice',
          text: 'A run was in progress, so the picture went in by name only — the model was not shown it.',
        });
      }
    } catch (e) {
      if (sessionKey !== key) return;
      pushEntry({ kind: 'notice', text: `send failed: ${e?.message ?? e}` });
    }
  }

  // Phase 4's upload half: the file lands in the session jail's inbox/, its
  // path is named in the message, and send() lists it so the server puts a
  // picture on the turn as pixels (ARCHITECTURE.md §Images).
  let fileInput = $state(null);
  // A count, not a flag: a drop can land while a picked upload is still
  // going, and the first to finish must not clear the other's spinner.
  let uploads = $state(0);
  const uploading = $derived(uploads > 0);
  let attachments = $state([]); // workspace-relative paths, announced on send

  const repeats = $derived(repeatedPictures(entries));
  // Pictures waiting behind the one drawing (`waitingPictures`).
  const queuedPictures = $derived(waitingPictures(entries, queue));

  const workspaceFile = (path) => `/api/chat/${key}/file?path=${encodeURIComponent(path)}`;

  // Download a generated picture: read and saved from a blob, so even an
  // incognito chat's picture leaves no address in the browser's history
  // (the reason its picture is not a link). Why one failed shows under it.
  let pictureNote = $state(null); // { path, why }
  async function savePicture(path) {
    const k = key;
    const why = await downloadPicture(fetch, workspaceFile(path), path);
    // A chat left — or an incognito chat ended (`forget` keeps the key and
    // sets `gone`) — while the download ran keeps no note of it (reviews of
    // #494).
    if (key !== k || gone) return;
    // Only this picture's note: another's failure is not cleared by this one
    // succeeding (review of #494).
    if (why) pictureNote = { path, why };
    else if (pictureNote?.path === path) pictureNote = null;
  }

  // Seed the input with the file to edit and leave the cursor after it.
  // Anything already typed is kept after the prefix, never replaced.
  // Save to library: the picture becomes a recurring character. The server
  // copies it in through the chat's jail and reads its seed from the
  // manifest beside it; this form only names and describes it. The lock box
  // starts checked when the picture was made from a locked character (the
  // owner's ruling: inherited by default, and a tap unchecks it) — and when
  // the answer cannot be had, it starts checked too, because there the safe
  // default and the fallback are the same side (review of #385). Save waits
  // until the answer is in. A password is optional: without one the Library
  // tab's lock is a plain toggle, so locking here always has a way back.
  let saving = $state(null);
  async function startSave(path) {
    saving = { path, name: '', description: '', locked: false, busy: false, msg: null, ready: false, inherited: false };
    try {
      const res = await fetch(`/api/library/source?key=${encodeURIComponent(key)}&path=${encodeURIComponent(path)}`);
      if (saving?.path !== path) return;
      if (!res.ok) throw new Error((await res.text()).trim());
      const src = await res.json();
      saving.inherited = !!src.suggest_locked;
      saving.locked = saving.inherited;
    } catch (e) {
      if (saving?.path !== path) return;
      saving.locked = true;
      saving.msg = `Could not check whether this picture used a locked character (${String(e?.message ?? e)}), so the lock box starts checked.`;
    } finally {
      if (saving?.path === path) saving.ready = true;
    }
  }
  async function saveToLibrary() {
    if (!saving) return;
    saving.busy = true;
    saving.msg = null;
    try {
      const res = await fetch('/api/library/save', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          key,
          path: saving.path,
          name: tameName(saving.name),
          description: saving.description.trim(),
          locked: saving.locked,
        }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const name = tameName(saving.name);
      saving = { ...saving, busy: false, msg: `Saved as ${name}. Name ${name} in a chat to draw them again.`, done: true };
    } catch (e) {
      saving.busy = false;
      saving.msg = String(e?.message ?? e);
    }
  }

  // The Edit button opens a modal where the owner paints the part to change
  // (EditModal.svelte). Anything already typed becomes its instruction.
  let editing = $state(null);
  function editImage(path) {
    editing = { path, src: workspaceFile(path), initial: draft.trim(), busy: false, error: null };
  }

  // A painted area goes up as a mask, named in the message and never put in
  // `attachments`: those ride on the turn as pixels, and a mask is for
  // `image_generate`, not for the model to look at (image-edit.js). The text
  // then goes out through `send`, like anything typed — steering a run in
  // progress, an incognito chat and a failure all behave the same.
  async function sendEdit({ text, mask }) {
    const sessionKey = key;
    const edit = editing;
    if (!edit) return;
    edit.busy = true;
    edit.error = null;
    try {
      let maskPath = null;
      if (mask) {
        const q = new URLSearchParams({ name: maskName(edit.path) });
        const res = await fetch(`/api/chat/${sessionKey}/upload?${q}`, { method: 'POST', body: mask });
        if (res.status === 410) {
          if (sessionKey === key) closeIncognito('closed');
          editing = null;
          return;
        }
        if (!res.ok) throw new Error((await res.text()).trim());
        maskPath = (await res.json()).path;
      }
      if (sessionKey !== key) return;
      const message = composeEditMessage(edit.path, maskPath, text);
      if (!message) {
        edit.busy = false; // never a modal that no button can close
        return;
      }
      draft = message;
      editing = null;
      await send();
    } catch (err) {
      if (editing === edit) {
        edit.busy = false;
        edit.error = `The mask could not be uploaded: ${err?.message ?? err}. Nothing was sent.`;
      }
    }
  }

  function uploadPicked(e) {
    const files = [...(e.target.files ?? [])];
    e.target.value = '';
    uploadFiles(files);
  }

  // Dropping is the button by another door: the same `uploadFiles`, so a
  // dropped file lands in inbox/ and is announced by path exactly as a
  // picked one is. The handlers sit on the window because Chat is only
  // mounted while the chat view is up, and because a file dropped anywhere
  // the page does not claim is *navigated to* by the browser — which throws
  // this page away, an incognito chat with it.
  let dragDepth = $state(0); // dragenter/leave fire at every child boundary
  // Not behind the edit modal: a file dropped there would join
  // `attachments` unseen and ride out on the modal's own send (review of #429).
  const canDrop = $derived(!gone && !voiceOpen && !editing);

  function onDragEnter(e) {
    if (!carriesFiles(e.dataTransfer)) return;
    e.preventDefault();
    dragDepth += 1;
  }

  function onDragOver(e) {
    if (!carriesFiles(e.dataTransfer)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = canDrop ? 'copy' : 'none';
  }

  function onDragLeave(e) {
    if (!carriesFiles(e.dataTransfer)) return;
    dragDepth = Math.max(0, dragDepth - 1);
  }

  function onDrop(e) {
    if (!carriesFiles(e.dataTransfer)) return;
    e.preventDefault();
    dragDepth = 0;
    if (!canDrop) return;
    const { files, folders } = droppedFiles(e.dataTransfer);
    for (const name of folders) {
      pushEntry({ kind: 'notice', text: `not attached: ${name} is a folder — drop the files inside it` });
    }
    uploadFiles(files);
  }

  async function uploadFiles(files) {
    const sessionKey = key;
    for (const f of files) {
      uploads += 1;
      try {
        const q = new URLSearchParams({ name: f.name });
        const res = await fetch(`/api/chat/${sessionKey}/upload?${q}`, { method: 'POST', body: f });
        if (res.status === 410) {
          // Only if it is still the chat on screen, as `send` checks: a
          // switch mid-upload must not strand an ordinary chat on the gone
          // screen (review of #326).
          if (sessionKey === key) closeIncognito('closed');
          return;
        }
        if (!res.ok) throw new Error((await res.text()).trim());
        const data = await res.json();
        // `attachments` belongs to whichever chat is on screen: a switch
        // mid-upload must not announce this chat's file in the next one — from
        // an incognito chat, into a recorded transcript (review of #326).
        if (sessionKey !== key) return;
        attachments.push(data.path);
      } catch (err) {
        if (sessionKey !== key) return;
        pushEntry({ kind: 'notice', text: `upload failed: ${err?.message ?? err}` });
      } finally {
        uploads -= 1;
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

  // A picture Stop, never the reply being spoken (ruling Q2, 2026-10-05).
  // With `call`, that one picture alone — a row's own Stop — and the rest of
  // the line stays; without, the chat's pictures (review of #606).
  async function stopPicture(call) {
    const one = typeof call === 'string' ? call : undefined;
    try {
      await fetch(`/api/chat/${key}/cancel`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ picture: true, call: one }),
      });
    } catch {
      // The picture's own result reports the outcome.
    }
  }

  // The queue panel's drag: the waiting pictures in a new order; the line
  // that comes back is the server's, refused order or not.
  async function reorderPictures(order) {
    const k = key;
    try {
      const res = await fetch(`/api/chat/${k}/jobs/order`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ order }),
      });
      const line = res.ok ? (await res.json()).queue : null;
      // A switch while the answer travelled: that line is the last chat's
      // (the #418 pattern; review of #607).
      if (line && key === k) queue = line;
    } catch {
      // The next `queue` event says how the line stands.
    }
  }

  const pct = $derived(
    usage?.window ? Math.min(100, Math.round((usage.prompt / usage.window) * 100)) : null
  );
  const fmt = (n) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));
</script>

<svelte:window
  ondragenter={onDragEnter}
  ondragover={onDragOver}
  ondragleave={onDragLeave}
  ondrop={onDrop}
/>

<div class="chat">
  {#if dragDepth > 0 && canDrop}
    <div class="drop-overlay" aria-hidden="true">
      <div class="drop-card">
        <svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="var(--accent-400)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.5l-8.2 8.2a5.5 5.5 0 01-7.8-7.8L13.6 4.3a3.7 3.7 0 015.2 5.2l-8.4 8.4a1.85 1.85 0 01-2.6-2.6l7.8-7.8" /></svg>
        <span>drop to attach — files land in this session's inbox/</span>
      </div>
    </div>
  {/if}
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
    <!-- Beside +, with its own icon (design §7): the same one tap away, and
         never mistaken for it. Only where incognito is on
         (FEATURES-DESIGN.md §5). -->
    {#if isShown(features.rows, 'incognito')}
    <button class="newbtn header incog" onclick={newIncognito} title="new incognito chat — nothing from it is kept" aria-label="new incognito chat">
      <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M3 10h18M6 10l1.6-4.2A1.5 1.5 0 019 5h6a1.5 1.5 0 011.4.8L18 10" /><circle cx="7.5" cy="15.5" r="2.5" /><circle cx="16.5" cy="15.5" r="2.5" /><path d="M10 15.5h4" /></svg>
    </button>
    {/if}
    <span class="title" title={incognito ? 'incognito' : key}>{incognito ? 'Incognito' : heading}</span>
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
      <ModelChip {model} {incognito} />
      {#if incognito && !gone}
        <button class="chip endchip" onclick={endIncognito} title="end this chat now — everything in it is deleted">End</button>
      {/if}
    </div>
  </header>

  {#if incognito && !gone}
    <!-- Does not scroll away (design §7), and carries §5.3's notice: shown
         before any search can happen, once per chat, in the page — never in
         the model's context. -->
    <div class="incog-banner" role="note">
      <span><strong>Incognito</strong> — nothing from this chat is kept. It ends when you tap End, or 30 minutes after this page is closed.</span>
      <span class="incog-search">A web search still reaches the search engine, which sees the query.</span>
    </div>
  {/if}

  {#snippet pastRow(h, archived)}
    <div class="dline">
      <button class="drow past" onclick={() => resumeSession(h.id)}>
        <!-- The name it earned, and the opening line for one that has
             not earned one yet (or was renamed past where the listing
             scan reads). -->
        <span class="dsnippet">{nameOf(h.title) || h.snippet}</span>
        <span class="dmeta">
          {#if h.kind === 'voice'}<span class="dkind">voice</span>{/if}
          {#if h.kind === 'task'}<span class="dkind">task</span>{/if}
          <!-- An archived row is dated by when it was put away: that is
               what a person looking through the archive remembers. -->
          {archived && h.archived_at ? `archived ${h.archived_at.slice(0, 10)}` : h.created_at.slice(0, 10)}
        </span>
      </button>
      <button class="dmore" class:open={menuFor === h.id} aria-label="archive or delete" aria-expanded={menuFor === h.id}
        onclick={() => (menuFor = menuFor === h.id ? null : h.id)}>⋯</button>
    </div>
    {#if menuFor === h.id}
      <div class="dactions">
        {#if archived}
          <button onclick={() => archiveChat(h.id, false)}>restore</button>
        {:else}
          <button onclick={() => archiveChat(h.id)}>archive</button>
        {/if}
        <button class="danger" onclick={() => deleteChat(h.id, nameOf(h.title) || h.snippet)}>delete…</button>
      </div>
    {/if}
  {/snippet}

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
        {#if isShown(features.rows, 'incognito')}
          <button class="newbtn incog" onclick={newIncognito} title="nothing from it is kept">incognito</button>
        {/if}
        <button class="newbtn" onclick={newSession}>
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M12 5v14M5 12h14" /></svg>
          new
        </button>
      </div>
      <div class="drawer-scroll">
        {#if drawerNote}
          <div class="dnote" role="status">
            <span>{drawerNote.text}</span>
            {#each drawerNote.left ?? [] as l}<span class="dleft">kept: {l}</span>{/each}
            <span class="dnote-actions">
              {#if drawerNote.retry}
                <button onclick={() => deleteChat(drawerNote.retry.id, drawerNote.retry.label)}>delete again</button>
              {/if}
              <button onclick={() => (drawerNote = null)}>dismiss</button>
            </span>
          </div>
        {/if}
        <div class="dsection">open</div>
        {#each rail.length ? rail : [{ key: 'main', running: false }] as s}
          <div class="dline">
            <button class="drow" class:dactive={s.key === key} onclick={() => { drawer = false; switchTo(s.key); }}>
              <span class="raildot" class:on={s.running}></span>
              <span class="dname">{s.incognito ? 'incognito chat' : sessionLabel(s)}</span>
              {#if s.incognito}<span class="dkind incog">incognito</span>{/if}
              {#if s.title?.startsWith('voice')}<span class="dkind">voice</span>{/if}
              {#if s.title?.startsWith('task: ')}<span class="dkind">task</span>{/if}
              {#if s.taint?.untrusted}<span class="railtaint">▲</span>{/if}
            </button>
            <!-- An incognito chat has its own End, and nothing to archive. -->
            {#if s.id && !s.incognito}
              <button class="dmore" class:open={menuFor === s.id} aria-label="archive or delete" aria-expanded={menuFor === s.id}
                onclick={() => (menuFor = menuFor === s.id ? null : s.id)}>⋯</button>
            {/if}
          </div>
          {#if s.id && menuFor === s.id}
            <div class="dactions">
              {#if s.running}
                <span class="dwhy">stop the run to archive or delete</span>
              {:else}
                <button onclick={() => archiveChat(s.id)}>archive</button>
                <button class="danger" onclick={() => deleteChat(s.id, sessionLabel(s))}>delete…</button>
              {/if}
            </div>
          {/if}
        {/each}
        <div class="dsection">earlier</div>
        {#if history === null}
          <div class="dempty">reading the record…</div>
        {:else}
          {#each history.filter((h) => !h.attached_key) as h}
            {@render pastRow(h, false)}
          {:else}
            <div class="dempty">nothing recorded yet</div>
          {/each}
        {/if}
        <button class="dsection dtoggle" onclick={toggleArchived} aria-expanded={archivedOpen}>
          archived {archivedOpen ? '▾' : '▸'}
        </button>
        {#if archivedOpen}
          {#if archivedRows === null}
            <div class="dempty">reading the archive…</div>
          {:else}
            {#each archivedRows.filter((h) => !h.attached_key) as h}
              {@render pastRow(h, true)}
            {:else}
              <div class="dempty">nothing archived</div>
            {/each}
          {/if}
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

  {#if gone}
    <!-- After End there is nothing to reopen (design §7): no "earlier"
         entry, no transcript. The page says so and offers a new one. -->
    <div class="gone" role="status">
      <p class="gone-head">
        {gone === 'ended' ? 'This incognito chat has ended.' : 'This incognito chat has closed.'}
      </p>
      <p class="gone-body">
        {gone === 'ended'
          ? 'Nothing from it was kept.'
          : 'It was idle for 30 minutes, or the server restarted. Nothing from it was kept.'}
      </p>
      {#if goneNote}<p class="gone-note">{goneNote}</p>{/if}
      <div class="gone-actions">
        <button class="newbtn incog" onclick={newIncognito}>new incognito chat</button>
        <button class="newbtn" onclick={() => switchTo(DEFAULT_KEY)}>back to chat</button>
      </div>
    </div>
  {:else}
  <div class="transcript" bind:this={transcriptEl}>
    {#if error}
      <div class="notice">{error}</div>
    {/if}
    {#each entries as entry, i}
      {#if entry.kind === 'user'}
        <div class="bubble" class:queued={entry.queued}>
          {entry.text}
          {#if entry.queued}<span class="queued-tag">{entry.delivery === 'discarded' ? 'not delivered — send again' : entry.delivery === 'delivered' ? 'steered' : 'queued'}</span>{/if}
          {#if entry.spoken}<span class="queued-tag">spoken</span>{/if}
        </div>
      {:else if entry.kind === 'assistant'}
        <!-- Download in every chat, incognito too (owner, 2026-10-01): the
             file is made in the browser from the reply on the page
             (`reply-export.js`), so the server keeps no trace, and a reply
             saved to the device is the owner's own act, like text copied
             out (INCOGNITO-DESIGN §1, R2's refinement). -->
        <div class="answer"><ChatProse text={entry.text} actions="mecha" listen={isShown(features.rows, 'calls') ? { chat: key, ...ownerVoice(), ...replyContext(entries, i) } : null} hidePictureRefs download /></div>
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
        {@const picture = pictureOf(entry)}
        {@const repeat = repeats.has(i)}
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
          {:else if entry.is_error}<span class="tool-state">failed</span>
          {:else if stillOut(entry)}<span class="tool-state">{queuedPictures.has(i) ? 'waiting its turn…' : 'drawing a picture…'}</span>{/if}
        </div>
        <!-- Still being drawn past the turn that asked for it (§5.4): it
             lands on this row when done. Its Stop ends the picture alone,
             never a reply that is running (review of #583). -->
        {#if stillOut(entry)}<button class="qmore" onclick={() => stopPicture(entry.id)}>Stop the picture</button>{/if}
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
            <!-- "still running", "the run ended first", "answered nothing"
                 and "answered this" are four different readings, and an
                 absent block would collapse the first three into the last. -->
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
            {:else if entry.unfinished}
              <div class="tsep">the run ended before this answered</div>
            {:else if !entry.blocked}
              <div class="tsep">answered with nothing</div>
            {/if}
            <!-- A picture already drawn above is not drawn inline again; it
                 is here, one tap away, for whoever wants to see it. -->
            {#if picture && repeat}
              {#if incognito}
                <span class="genimg inpanel"><img src={workspaceFile(picture)} alt="viewed" loading="lazy" /></span>
              {:else}
                <a class="genimg inpanel" href={workspaceFile(picture)} target="_blank" rel="noopener">
                  <img src={workspaceFile(picture)} alt="viewed" loading="lazy" />
                </a>
              {/if}
            {/if}
          </div>
        {/if}
        <!-- Outside the disclosure: the picture is the answer, not a detail
             of the call. Served from this session's own jail, images only
             (serve/files.rs), so a tap opens it full size. -->
        {#if picture && !repeat}
          {#if incognito}
            <!-- No link in an incognito chat: opening the picture in a tab
                 writes its address into the browser's history, which
                 outlives the chat (R6). -->
            <span class="genimg"><img src={workspaceFile(picture)} alt="generated" loading="lazy" /></span>
          {:else}
            <a class="genimg" href={workspaceFile(picture)} target="_blank" rel="noopener">
              <img src={workspaceFile(picture)} alt="generated" loading="lazy" />
            </a>
          {/if}
          <!-- Opens the edit modal (EditModal.svelte): paint what may change,
               say what to change. The path is what lets the model pass the
               right file as the reference. -->
          <button class="genedit" onclick={() => editImage(picture)}>Edit</button>
          <button class="genedit" onclick={() => savePicture(picture)}>Download</button>
          {#if pictureNote?.path === picture}<span class="genfail">not downloaded: {pictureNote.why}</span>{/if}
          {#if !incognito}
            <!-- Not in an incognito chat: saving writes outside the room. -->
            <button class="genedit" onclick={() => (saving?.path === picture ? (saving = null) : startSave(picture))}>Save to library</button>
            {#if saving?.path === picture}
              <form class="libsave" onsubmit={(e) => { e.preventDefault(); saveToLibrary(); }}>
                {#if !saving.done}
                  <input placeholder="name, e.g. maya" bind:value={saving.name} autocomplete="off" />
                  <textarea rows="2" placeholder="a short description — include build and height" bind:value={saving.description}></textarea>
                  <label class="libsave-lock">
                    <input type="checkbox" bind:checked={saving.locked} />
                    lock (hide while browsing)
                    {#if saving.inherited}<span class="libsave-why">— made from a locked character</span>{/if}
                  </label>
                  <button class="genedit" disabled={!saving.ready || saving.busy || !validName(tameName(saving.name)) || !saving.description.trim()}>Save</button>
                {/if}
                {#if saving.msg}<div class="libsave-msg">{saving.msg}</div>{/if}
              </form>
            {/if}
          {/if}
        {/if}
      {:else if entry.kind === 'notice'}
        <div class="notice">{entry.text}</div>
      {:else if entry.kind === 'draft'}
        {@const d = entry.draft}
        {@const sum = rowSummary(d)}
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
          <!-- A reply's arguments are a thread id and prose: say who and what
               from the thread it read, as the outbox does — a sender only
               from a verified read (outbox-view.js, rowSummary). -->
          {#if d.headline || sum?.subject}<div class="dheadline">{d.headline || sum.subject}</div>{/if}
          {#if sum?.who && !d.headers.some(([k]) => k === 'to')}
            <div class="dfield"><span class="dkey">replying to</span><span>{sum.who}</span></div>
          {/if}
          {#each d.headers as [name, value]}
            <div class="dfield"><span class="dkey">{name}</span><span>{value}</span></div>
          {/each}
          {#if d.body}<div class="dbody">{d.body}</div>{/if}
          {#each d.other.filter(([k, v]) => !ROUTING_KEYS.includes(k) || (k === 'reply_all' && v === 'true')) as [name, value]}
            <div class="dfield"><span class="dkey">{name}</span><span>{value}</span></div>
          {/each}
          <!-- Every argument stays reachable (DraftView's guarantee): the
               routing ids folded above are one click away, as in the outbox. -->
          {#if d.other.some(([k]) => ROUTING_KEYS.includes(k))}
            <button class="dtoggle" onclick={() => (entry.showArgs = !entry.showArgs)}>
              {entry.showArgs ? 'hide' : 'show'} the exact arguments
            </button>
            {#if entry.showArgs}
              {#each d.other.filter(([k, v]) => ROUTING_KEYS.includes(k) && !(k === 'reply_all' && v === 'true')) as [name, value]}
                <div class="dfield"><span class="dkey">{name}</span><span>{value}</span></div>
              {/each}
            {/if}
          {/if}
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
      <div class="answer"><ChatProse text={streaming} hidePictureRefs /></div>
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
    <PictureQueue items={queue} oncancel={(id) => stopPicture(id)} onreorder={reorderPictures} />
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
      <!-- In an incognito chat too: the worker holds its log silence for
           the call and vouches for it, and the server admits a spoken turn
           into the chat only on that word (design §3.4). Only where calls
           are on (FEATURES-DESIGN.md §5). -->
      {#if isShown(features.rows, 'calls')}
      <button
        class="round voice"
        onclick={startVoice}
        title={incognito ? 'start a voice call — nothing from it is kept' : 'start a voice call in this conversation'}
      >
        <svg viewBox="0 0 24 24" width="19" height="19" fill="none" stroke="var(--accent-400)" stroke-width="1.8" stroke-linecap="round"><path d="M4 10v4M8 7v10M12 4v16M16 7v10M20 10v4" /></svg>
      </button>
      {/if}
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
  {/if}

  {#if voiceOpen}
    <div class="voice-overlay">
      <div class="voice-top">
        {#if vIncognito}
          <span class="chip incog">speaking into this incognito chat — nothing from the call is kept</span>
        {:else}
          <span class="chip">speaking into {vKey === 'main' ? 'your chat' : `“${vKey}”`} — same conversation, same memory</span>
        {/if}
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
        <div class="meter" class:paused={vState.name === 'paused'} title="your microphone, live">
          {#each Array(14) as _, i}
            <span
              class="tick"
              class:lit={vLevel * 14 > i}
              style:height="{6 + Math.abs(i - 6.5) * -0 + (i % 2 ? 6 : 0) + 8}px"
            ></span>
          {/each}
        </div>
      </div>
      <div class="voice-pane" bind:this={voicePane} onscroll={vScrolled}>
        {#each vTranscript as line}
          {#if line.who === 'user'}
            <div class="vbubble" class:vlost={line.undelivered}>{line.text}{#if line.undelivered}<span class="queued-tag">not delivered — send again</span>{/if}</div>
          {:else if line.picture}
            <!-- The picture itself, as the chat draws it (the owner's ask,
                 2026-10-06). Not a link: opening a tab on a phone drops the
                 call. -->
            <!-- No height until it loads: follow the bottom again once it
                 does (review of #576). -->
            <span class="vanswer vshot"><img src={workspaceFile(line.picture)} alt={line.text} loading="lazy" onload={() => vStick && voicePane?.scrollTo({ top: voicePane.scrollHeight })} /></span>
          {:else if line.making}
            <!-- Still being drawn: its place in the call, and a Stop for the
                 picture alone, which leaves the reply being spoken (ruling Q2). -->
            <span class="vanswer vmaking">{line.text} <button class="qmore" onclick={() => stopPicture()}>Stop</button></span>
          {:else}
            <!-- The chat's own renderer, so a call formats a reply the way
                 the chat does (the owner's ask, 2026-10-06). -->
            <div class="vanswer"><ChatProse text={line.text} hidePictureRefs /></div>
          {/if}
        {/each}
        {#if vReplying}
          <div class="vanswer"><ChatProse text={vReplying} hidePictureRefs /></div>
        {/if}
        {#each vSpeaking as entry}
          {#if entry.who === 'notice'}
            <div class="vanswer vnote">{entry.text}</div>
          {:else if entry.who === 'user'}
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
      <form class="typerow" onsubmit={(e) => { e.preventDefault(); sendTyped(); }}>
        <input
          class="typebox"
          placeholder="Type instead of speaking"
          aria-label="Type into the call"
          bind:value={vTyped}
          onfocus={typingStart}
          onblur={typingEnd}
          disabled={!vLinked}
          maxlength="4000"
        />
        <button type="submit" class="typesend" aria-label="send" disabled={!vLinked || !vTyped.trim()}>
          <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 19V5M6 11l6-6 6 6" /></svg>
        </button>
      </form>
      {#if vTyping && !vMuted}<div class="typehint">mic paused while you type</div>{/if}
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

{#if editing}
  <EditModal
    src={editing.src}
    path={editing.path}
    initial={editing.initial}
    busy={editing.busy}
    error={editing.error}
    onsend={sendEdit}
    onclose={() => (editing = null)}
  />
{/if}

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
  /* Incognito's own colour is the muted text rather than the accent: the
     accent is mecha's voice, and this is the chat it does not remember. An
     outline, no fill, so it reads as a different door rather than a warning. */
  .newbtn.header.incog {
    margin: 0 4px 0 6px;
  }
  .newbtn.incog {
    color: var(--text-muted);
    background: var(--bg);
    border-color: var(--text-muted);
  }
  .newbtn.incog:hover {
    color: var(--text);
    border-color: var(--text);
  }
  .endchip {
    cursor: pointer;
    color: var(--text);
    background: var(--bg);
    border-color: var(--text-muted);
    min-height: 28px;
  }
  .incog-banner {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0 var(--gutter-gear) 6px var(--gutter);
    padding: 8px 12px;
    font-size: 13px;
    color: var(--text);
    border: 1px dashed var(--text-muted);
    border-radius: var(--radius);
    flex-shrink: 0;
  }
  .incog-search {
    color: var(--text-muted);
    font-size: 12px;
  }
  .gone {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 24px var(--gutter);
    text-align: center;
  }
  .gone-head {
    margin: 0;
    font-size: 17px;
    font-weight: 500;
  }
  .gone-body {
    margin: 0;
    color: var(--text-muted);
    font-size: 14px;
  }
  .gone-note {
    margin: 0;
    color: var(--hazard);
    font-size: 13px;
  }
  .gone-actions {
    display: flex;
    gap: 8px;
    margin-top: 8px;
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
  .dkind.incog {
    color: var(--text-muted);
    background: var(--bg);
    border: 1px solid var(--text-muted);
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
  .dline {
    display: flex;
    align-items: stretch;
    gap: 2px;
  }
  .dline .drow {
    flex: 1;
    min-width: 0;
  }
  .dmore {
    flex: 0 0 36px;
    background: none;
    border: none;
    border-radius: var(--radius);
    color: var(--text-muted);
    font-size: 16px;
    cursor: pointer;
  }
  .dmore:hover,
  .dmore.open {
    color: var(--text);
    background: var(--accent-900);
  }
  /* A pointer that can hover gets the row's own quiet: the control appears
     under it. A finger cannot hover, so on touch it simply stays. */
  @media (hover: hover) {
    .dline .dmore {
      opacity: 0;
    }
    .dline:hover .dmore,
    .dmore:focus-visible,
    .dmore.open {
      opacity: 1;
    }
  }
  .dactions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 2px 10px 10px;
  }
  .dactions button,
  .dnote button {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text);
    background: none;
    border: 1px solid var(--accent-700);
    border-radius: var(--radius-chip);
    padding: 6px 10px;
    min-height: 32px;
    cursor: pointer;
  }
  .dactions button.danger {
    color: var(--hazard);
    border-color: var(--hazard);
  }
  .dwhy {
    font-size: 12px;
    color: var(--text-muted);
  }
  /* Spelled out rather than inherited from `.dsection`: the global button
     reset outranks a lone class, and a toggle that sits 10px off its
     neighbours in another colour reads as a different kind of thing. */
  .drawer-scroll .dtoggle {
    display: block;
    width: 100%;
    text-align: left;
    background: none;
    border: none;
    cursor: pointer;
    margin-top: 8px;
    font-family: var(--mono);
    font-size: 10px;
    color: var(--accent-700);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    padding: 12px 10px 6px;
  }
  .dnote {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--text);
    background: var(--surface);
    border-radius: var(--radius);
    padding: 10px;
    margin: 4px 2px 8px;
    overflow-wrap: anywhere;
  }
  .dleft {
    color: var(--text-muted);
  }
  .dnote-actions {
    display: flex;
    gap: 6px;
    margin-top: 4px;
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
  .genimg {
    display: block;
    margin: 4px 0 8px 18px;
    max-width: min(100%, 512px);
  }
  .libsave { display: flex; flex-direction: column; gap: 8px; margin-top: 8px; max-width: 420px; }
  .libsave input:not([type='checkbox']), .libsave textarea { background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 14px; padding: 9px 11px; }
  .libsave-lock { display: flex; align-items: center; gap: 8px; font-size: 12px; color: var(--text-muted); flex-wrap: wrap; }
  .libsave-why { color: var(--hazard); font-family: var(--mono); font-size: 11px; }
  .libsave-msg { font-size: 12px; color: var(--text-muted); line-height: 1.45; }
  .genfail { font-size: 12px; color: var(--hazard); }
  .genedit {
    align-self: flex-start;
    margin: -4px 0 10px 18px;
    padding: 4px 12px;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--accent-400);
    background: none;
    border: 1px solid var(--accent-400);
    border-radius: 999px;
    cursor: pointer;
  }
  .genimg.inpanel {
    margin: 0;
  }
  .genimg img {
    display: block;
    width: 100%;
    height: auto;
    border-radius: 8px;
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
    /* A goal put beside a question arrives as two paragraphs; collapsing
       the break would run the hypothesis into the question it introduces. */
    white-space: pre-line;
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
  /* Over the whole window, not just the chat column: the drop handlers are
     on the window, so the target the page shows is the target it has.
     No pointer events, so the drag keeps landing on the page beneath and
     the enter/leave count stays the page's own. */
  .drop-overlay {
    position: fixed;
    inset: 0;
    z-index: 60;
    pointer-events: none;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--gutter);
    background: color-mix(in srgb, var(--void) 72%, transparent);
    outline: 2px dashed var(--accent-500);
    outline-offset: -12px;
  }
  .drop-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    padding: 20px 24px;
    border-radius: var(--radius);
    background: var(--accent-900);
    border: 1px solid var(--accent-700);
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text);
    text-align: center;
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
  /* The incognito door's own outline (`.newbtn.incog`), so the call says
     which kind of chat it is speaking into before a word is said. */
  .voice-top .chip.incog {
    color: var(--text-muted);
    background: var(--bg);
    border-color: var(--text-muted);
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
  /* Paused is the link, not the call: the slot goes dark rather than to a
     colour, and the meter dims with it so a lit ring cannot say "heard"
     over a line the page knows is carrying nothing. Never hazard amber -
     brand.md keeps that for lines and ticks, and a pause is a wait, not a
     fault. */
  .slot.paused {
    fill: var(--accent-900);
  }
  .meter.paused {
    opacity: 0.3;
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
  /* A line the chat dropped: shown, and said so, since the call covers the
     chat's own tag. */
  .vbubble.vlost {
    color: var(--text-muted);
  }
  .vbubble {
    white-space: pre-wrap;
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
  /* A picture in the transcript, drawn where it was made. */
  .vmaking { font-family: var(--mono); font-size: 12px; color: var(--text-muted); }
  .vshot {
    display: block;
    width: min(92%, 320px);
  }
  .vshot img {
    display: block;
    width: 100%;
    height: auto;
    border-radius: 8px;
  }
  /* The call's own state — a dead mic, a dropped line — never speech. */
  .vnote {
    color: var(--text-muted);
    font-style: italic;
  }
  .typerow {
    display: flex;
    gap: 8px;
    margin: 14px 20px 0;
  }
  .typebox {
    flex: 1;
    min-width: 0;
    min-height: 44px;
    padding: 0 14px;
    border-radius: 22px;
    border: 1px solid var(--accent-900);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 16px;
  }
  .typebox:focus {
    outline: none;
    border-color: var(--accent-500);
  }
  .typesend {
    flex-shrink: 0;
    width: 44px;
    height: 44px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: var(--accent-400);
    color: var(--void);
    border: none;
    cursor: pointer;
  }
  .typesend:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .typehint {
    margin: 6px 20px 0;
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-muted);
  }
  .voice-controls {
    display: flex;
    justify-content: center;
    align-items: center;
    gap: 24px;
    padding: 16px 0 34px;
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
