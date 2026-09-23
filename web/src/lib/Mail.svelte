<script>
  import { onDestroy, untrack } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';
  import { apiFetch as fetch } from './api.js';
  import { parseThread } from './mail-thread.js';
  import { LANES, sortRows, acceptVerb, keyOf, senderOf, sweepGroups, ageOf, tickedGroups } from './mail-desk.js';
  import { MailQueue, HOLD_MS, VERB_PAST, VERB_LABEL, UNSEEN } from './mail-queue.svelte.js';

  // Mail on a phone: the desk's model with thumbs instead of keys. The queue,
  // its hold-then-commit timing and its reads are MailQueue's, shared with
  // MailDesk.svelte; this file is the phone's layout and gestures.
  //
  // - Swipe a row right to accept its suggestion, left to archive. Tap to
  //   read. A row the classifier had no suggestion for springs back from a
  //   right swipe and says so, rather than guessing.
  // - The thread view leads with one big Accept, and acting on a thread opens
  //   the next one in place: triage is a run of threads, not a trip back to
  //   the list after each.
  // - Every action is held for a few seconds behind an Undo button, then
  //   commits. Replies, forwards and invites stage into the outbox; nothing
  //   sends from here. Spam is the one verb that confirms first — it trains
  //   the provider's filter, the only effect outside the owner's mailbox.

  const INBOX = { id: 'inbox', label: 'Inbox' };
  const SWIPE_AT = 90; // px past which a released swipe acts
  const SWIPE_MAX = 140;

  let lane = $state('respond');
  let screen = $state('list'); // list | thread | sweep
  let cursor = $state(0);
  let asking = $state(null); // { verb, label, placeholder, wantTo, required, rows }
  let askText = $state('');
  let askTo = $state('');
  let confirmSpam = $state(null);
  let more = $state(false); // the thread view's second row of actions
  let composing = $state(false);
  let cTo = $state('');
  let cSubject = $state('');
  let cBody = $state('');
  let cAccount = $state('');
  let busy = $state(false);
  let toast = $state(null); // { text, undo }
  let toastTimer;

  const q = new MailQueue({ say: (text, undo) => say(text, undo), keepKey: () => (cur ? keyOf(cur) : null) });
  q.attach({ inboxOpen: () => lane === 'inbox' });
  onDestroy(() => clearTimeout(toastTimer));

  function say(text, undo = false) {
    toast = { text, undo };
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), undo ? HOLD_MS : 4000);
  }

  // ---- what is on screen ----

  const laneRows = $derived(q.laneRows(lane));
  const visible = $derived(lane === 'inbox' ? laneRows : sortRows(laneRows));
  const at = $derived(Math.max(0, Math.min(cursor, visible.length - 1)));
  const cur = $derived(screen === 'thread' ? (visible[at] ?? null) : null);
  const curRead = $derived(cur ? q.reads.get(keyOf(cur)) : null);
  const parsed = $derived(curRead?.status === 'ok' ? parseThread(curRead.text) : null);
  const leftCount = $derived(Object.values(q.laneCounts).reduce((a, b) => a + b, 0));

  // Acting on the last thread of a lane leaves nothing to show: back to the
  // (now empty) list rather than a blank reader.
  $effect(() => {
    if (screen === 'thread' && q.rows !== null && visible.length === 0) screen = 'list';
  });

  // Read the open thread and the next one; on the list, the top row, so the
  // first tap opens a thread that is already loaded.
  $effect(() => {
    const here = screen === 'thread' ? cur : screen === 'list' ? visible[0] : null;
    if (!here) return;
    const ahead = screen === 'thread' ? [visible[at + 1]] : [];
    untrack(() => q.readAround(here, ahead));
  });

  function pickLane(id) {
    lane = id;
    cursor = 0;
    if (id === 'inbox' && q.inbox === null) q.loadInbox();
  }

  function openAt(i) {
    cursor = i;
    more = false;
    screen = 'thread';
  }

  // ---- acting ----

  function label(verb, list) {
    return list.length === 1
      ? `${VERB_PAST[verb]} · ${list[0].subject || list[0].summary}`
      : `${VERB_PAST[verb]} ${list.length} threads`;
  }

  function run(verb, list, extra = {}) {
    if (!list.length) return false;
    more = false;
    return q.hold(list.map((row) => ({ row, verb, extra })), label(verb, list));
  }

  function accept(row) {
    const verb = acceptVerb(row);
    if (verb === 'spam') {
      confirmSpam = row;
      return;
    }
    if (!verb) {
      say('No suggestion to accept — pick an action');
      return;
    }
    run(verb, [row]);
  }

  function ask(verb, text, placeholder, row, { wantTo = false, required = false } = {}) {
    more = false;
    // Before the owner writes anything: none of the asking verbs works on a
    // thread the store has never seen.
    if (!q.canAct(verb, [row])) {
      say(UNSEEN);
      return;
    }
    // The thread is fixed now: the minute's reload must not retarget it.
    asking = { verb, label: text, placeholder, wantTo, required, rows: [row] };
    askText = '';
    askTo = '';
  }

  function submitAsk() {
    const a = asking;
    if (!a) return;
    if (a.wantTo && !askTo.trim()) return;
    if (a.required && !askText.trim()) return;
    const extra = {};
    if (askText.trim()) extra.text = askText.trim();
    if (a.wantTo) extra.to = askTo.trim();
    asking = null;
    run(a.verb, a.rows, extra);
  }

  function markSpam() {
    const row = confirmSpam;
    confirmSpam = null;
    run('spam', [row]);
  }

  async function stageCompose() {
    if (!cTo.trim() || !cSubject.trim() || !cBody.trim()) return;
    busy = true;
    try {
      const res = await fetch('/api/mail/compose', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ to: cTo.trim(), subject: cSubject.trim(), body: cBody, account: cAccount.trim() || null }),
      });
      const text = await res.text();
      if (!res.ok) throw new Error(text.trim());
      composing = false;
      cTo = cSubject = cBody = cAccount = '';
      say('Staged — review it in the Outbox before it sends');
      q.error = null;
    } catch (e) {
      q.error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  // ---- swiping ----
  //
  // Pointer events, so a mouse drag in a narrow desktop window works like a
  // finger. `touch-action: pan-y` on the row leaves vertical scrolling to the
  // browser; a drag that starts out vertical is abandoned at once, so a
  // scroll never turns into an archive.

  // One gesture at a time, owned by the pointer that started it: a second
  // finger landing mid-swipe is ignored rather than retargeting the first.
  let drag = $state(null); // { id, key, x0, y0, dx, moved }
  let swallowClick = false;

  function down(e, row) {
    if (e.pointerType === 'mouse' && e.button !== 0) return;
    // A gesture whose row was unmounted mid-drag (the minute's reload) may
    // never see its pointerup or pointercancel; do not let it disable
    // swiping for the session. A second finger still defers to a live one.
    if (drag && visible.some((r) => keyOf(r) === drag.key)) return;
    drag = { id: e.pointerId, key: keyOf(row), x0: e.clientX, y0: e.clientY, dx: 0, moved: false };
    e.currentTarget.setPointerCapture?.(e.pointerId);
  }

  function moveDrag(e) {
    if (!drag || e.pointerId !== drag.id) return;
    const dx = e.clientX - drag.x0;
    const dy = e.clientY - drag.y0;
    if (!drag.moved && Math.abs(dy) > 10 && Math.abs(dy) > Math.abs(dx)) {
      drag = null;
      return;
    }
    if (Math.abs(dx) > 8) drag.moved = true;
    drag.dx = Math.max(-SWIPE_MAX, Math.min(SWIPE_MAX, dx));
  }

  function up(e, row) {
    if (!drag || e.pointerId !== drag.id || drag.key !== keyOf(row)) return;
    const d = drag;
    drag = null;
    swallowClick = d.moved;
    if (d.dx > SWIPE_AT) accept(row);
    else if (d.dx < -SWIPE_AT) run('archive', [row]);
  }

  function tap(i) {
    if (swallowClick) {
      swallowClick = false;
      return;
    }
    openAt(i);
  }

  const dxOf = (row) => (drag && drag.key === keyOf(row) ? drag.dx : 0);

  // ---- sweep ----

  const SWEEP_VERBS = ['archive', 'reply', 'task', 'schedule'];
  const SWEEP_LABEL = { archive: 'Archive', reply: 'Draft replies', task: 'Make tasks', schedule: 'Draft invites' };
  let sweepVerb = $state('archive');
  const sweepMarks = new SvelteSet(); // groups toggled; what that means is `tickedGroups`'s
  const sweepRows = $derived(q.laneRows('respond').concat(q.laneRows('notify')));
  const groups = $derived(sweepGroups(sweepRows, sweepVerb));
  const sweepCounts = $derived(
    Object.fromEntries(SWEEP_VERBS.map((v) => [v, sweepRows.filter((r) => r.proposed === v).length])),
  );
  let sweepSeen = $state(new Set()); // groups on screen when the sheet opened, for `tickedGroups`
  let sweepSeenRows = $state(new Set()); // and the threads, so a late arrival cannot join a ticked group
  const ticked = $derived(new Set(tickedGroups(groups, sweepVerb, sweepMarks, sweepSeen).map((g) => g.key)));
  // Only threads there when the sheet opened, as on the desk: the count on
  // the button must not move under a thumb because the minute's reload ran.
  const checkedRows = $derived(
    groups.filter((g) => ticked.has(g.key)).flatMap((g) => g.rows).filter((r) => sweepSeenRows.has(keyOf(r))),
  );

  // Archive and task sweeps open with every group on screen ticked, drafting
  // sweeps with none; a group that arrives while the sheet is open starts
  // unticked either way (`tickedGroups`) — an archive has no inverse past
  // its hold, and each drafting thread is an agent run.
  function setSweepVerb(v) {
    sweepVerb = v;
    sweepMarks.clear();
    sweepSeen = new Set(sweepGroups(sweepRows, v).map((g) => g.key));
    sweepSeenRows = new Set(sweepRows.map(keyOf));
  }

  function openSweep() {
    setSweepVerb('archive');
    screen = 'sweep';
  }

  function applySweep() {
    const n = checkedRows.length;
    if (!n) return;
    if (q.hold(checkedRows.map((row) => ({ row, verb: sweepVerb, extra: {} })), `${VERB_PAST[sweepVerb]} ${n} threads from the sweep`)) {
      screen = 'list';
    }
  }

  function toggleGroup(g) {
    if (sweepMarks.has(g.key)) sweepMarks.delete(g.key);
    else sweepMarks.add(g.key);
  }

  const urgencyClass = (u) => (u === 'now' || u === 'today' || u === 'week' ? `u-${u}` : '');
</script>

{#snippet hazardGlyph(size = 13)}
  <svg viewBox="0 0 24 24" width={size} height={size} style="flex-shrink: 0" fill="none" stroke="var(--hazard)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 4l9 16H3z" /><path d="M12 11v4M12 17.5v.5" />
  </svg>
{/snippet}

{#snippet chevron(dir)}
  <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
    {#if dir === 'left'}<path d="M15 6l-6 6 6 6" />{:else if dir === 'up'}<path d="M18 15l-6-6-6 6" />{:else}<path d="M6 9l6 6 6-6" />{/if}
  </svg>
{/snippet}

<div class="page">
  {#if screen === 'list'}
    <header class="listhead">
      <div class="titlerow">
        <span class="title">Mail</span>
        {#if q.rows !== null}<span class="left">{leftCount} left</span>{/if}
      </div>
      <div class="lanes" role="tablist" aria-label="Lanes">
        {#each LANES as l}
          <button class="lanechip" class:on={lane === l.id} role="tab" aria-selected={lane === l.id} onclick={() => pickLane(l.id)}>
            {l.label} <span class="n">{q.laneCounts[l.id] ?? 0}</span>
          </button>
        {/each}
        <button class="lanechip" class:on={lane === 'inbox'} role="tab" aria-selected={lane === 'inbox'} onclick={() => pickLane('inbox')}>
          {INBOX.label}{#if q.inbox} <span class="n">{q.inboxRows.length}</span>{/if}
        </button>
      </div>
    </header>

    <div class="scroll">
      {#if q.error}<div class="warnline">{@render hazardGlyph()}<span>{q.error}</span></div>{/if}
      {#if (lane === 'respond' || lane === 'notify') && sweepCounts.archive > 1}
        <button class="sweepcard" onclick={openSweep}>
          <span class="grow">
            <strong>{sweepCounts.archive} suggested archives</strong>
            <span>Clear them by sender group</span>
          </span>
          <span class="sweepgo">Sweep</span>
        </button>
      {/if}

      {#if !q.error && ((lane !== 'inbox' && q.rows === null) || (lane === 'inbox' && q.inbox === null))}
        <div class="empty">{lane === 'inbox' ? 'reading the inbox — every account, newest first…' : 'reading the queue…'}</div>
      {:else}
        <div class="rows">
          {#each visible as r, i (keyOf(r))}
            {@const dx = dxOf(r)}
            <div class="swipe">
              <div class="under" class:accepting={dx > 0} class:archiving={dx < 0}>
                <span style:opacity={dx > 0 ? Math.min(1, dx / SWIPE_AT) : 0}>
                  {acceptVerb(r) ? `Accept · ${VERB_LABEL[acceptVerb(r)].toLowerCase()}` : 'No suggestion'}
                </span>
                <span class="grow"></span>
                <span style:opacity={dx < 0 ? Math.min(1, -dx / SWIPE_AT) : 0}>Archive</span>
              </div>
              <button
                class="row"
                class:dragging={drag && drag.key === keyOf(r)}
                style:transform="translateX({dx}px)"
                onpointerdown={(e) => down(e, r)}
                onpointermove={moveDrag}
                onpointerup={(e) => up(e, r)}
                onpointercancel={(e) => { if (drag && e.pointerId === drag.id) drag = null; }}
                onclick={() => tap(i)}
              >
                <span class="urg {urgencyClass(r.urgency)}"></span>
                <span class="rowbody">
                  <span class="line1">
                    {#if r.unread}<span class="unread"></span>{/if}
                    <span class="from">{senderOf(r)}</span>
                    {#if q.failed.has(keyOf(r))}<span class="chip bad">failed</span>{/if}
                    {#if r.state === 'failed'}<span class="chip bad">not classified</span>{/if}
                    <span class="grow"></span>
                    <span class="age">{ageOf(r.date)}</span>
                  </span>
                  <span class="subject">{r.subject || r.summary}</span>
                  <span class="line3">
                    <span class="summary">{r.subject && r.summary !== r.subject ? r.summary : ''}</span>
                    {#if r.deadline}<span class="chip due">due {r.deadline}</span>{/if}
                    {#if acceptVerb(r)}<span class="prop p-{r.proposed}">{VERB_LABEL[acceptVerb(r)].toLowerCase()}</span>{/if}
                  </span>
                </span>
              </button>
            </div>
          {:else}
            <div class="empty">{lane === 'inbox' ? (q.inboxNote ?? 'The inbox is empty.') : 'Lane clear.'}</div>
          {/each}
        </div>
      {/if}
    </div>

    {#if visible.length && lane !== 'inbox'}
      <div class="hint">swipe → accept · ← archive · tap to read</div>
    {/if}
    <button class="fab" onclick={() => (composing = true)} aria-label="Write a new email — staged for review">
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="var(--void)" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 20h9M16.5 3.5a2.1 2.1 0 013 3L7 19l-4 1 1-4z" /></svg>
    </button>
  {:else if screen === 'thread' && cur}
    <header class="threadhead">
      <button class="iconbtn" onclick={() => (screen = 'list')} aria-label="Back to the list">{@render chevron('left')}</button>
      <span class="pos">{at + 1} of {visible.length}</span>
      <span class="grow"></span>
      <button class="iconbtn" disabled={at === 0} onclick={() => { cursor = at - 1; more = false; }} aria-label="Previous thread">{@render chevron('up')}</button>
      <button class="iconbtn" disabled={at >= visible.length - 1} onclick={() => { cursor = at + 1; more = false; }} aria-label="Next thread">{@render chevron('down')}</button>
    </header>
    <div class="scroll reader">
      {#if q.error}<div class="warnline">{@render hazardGlyph()}<span>{q.error}</span></div>{/if}
      <h1>{cur.subject || cur.summary}</h1>
      <div class="meta">
        <span class="who">{senderOf(cur)}</span>
        <!-- The address as well as the name: the name is the sender's own
             choice, and this is the page where one swipe accepts. -->
        {#if cur.from_name && cur.from}<span class="addr">&lt;{cur.from}&gt;</span>{/if}
        <span>· {cur.account}</span>
        {#if cur.date}<span>· {ageOf(cur.date)} ago</span>{/if}
        {#if cur.deadline}<span class="chip due">due {cur.deadline}</span>{/if}
      </div>
      {#if q.failed.has(keyOf(cur))}
        <div class="warnline">{@render hazardGlyph()}<span>The last action on this thread failed: {q.failed.get(keyOf(cur))}</span></div>
      {/if}
      {#if acceptVerb(cur)}
        <div class="suggest">
          <span class="kicker">Suggested</span>
          <span><strong>{VERB_LABEL[acceptVerb(cur)]}</strong>{#if cur.subject && cur.summary && cur.summary !== cur.subject}<span>{' — '}{cur.summary}</span>{/if}</span>
        </div>
      {/if}
      {#if !curRead || curRead.status === 'loading'}
        <div class="empty">reading the thread…</div>
      {:else if curRead.status === 'error'}
        <div class="warnline">{@render hazardGlyph()}<span>Could not read this thread: {curRead.text}</span></div>
      {:else if parsed}
        {#each parsed.messages as msg}
          <div class="msg">
            <div class="msg-meta">{msg.meta}</div>
            {#if msg.subject && msg.subject !== cur.subject}<div class="msg-subject">{msg.subject}</div>{/if}
            <!-- Third-party text: the gutter marks every line, the outbox
                 source-read rule — a heading scrolls off, a per-line marker
                 cannot. Plain text on purpose: a rendered link in a
                 stranger's mail is a tap onto a stranger's URL. -->
            <div class="quoted"><span class="gutter"></span><div class="mailbody">{msg.body}</div></div>
          </div>
        {/each}
      {:else}
        <div class="quoted"><span class="gutter"></span><div class="qtext">{curRead.text}</div></div>
      {/if}
    </div>
    <div class="actions">
      {#if acceptVerb(cur)}
        <button class="bigaccept" onclick={() => accept(cur)}>Accept · {VERB_LABEL[acceptVerb(cur)]}</button>
      {/if}
      {#if more}
        <div class="grid">
          <button class="abtn" onclick={() => ask('schedule', 'Steering for the invite (optional)', 'propose Thursday afternoon', cur)}>Schedule…</button>
          <button class="abtn" onclick={() => ask('forward', 'Forward to (comma-separated) + covering note', 'FYI — this is the one I mentioned', cur, { wantTo: true })}>Forward…</button>
          <button class="abtn" onclick={() => run('dismiss', [cur])}>Dismiss</button>
          <button class="abtn" onclick={() => { more = false; confirmSpam = cur; }}>Spam…</button>
        </div>
      {/if}
      <div class="grid">
        <button class="abtn" onclick={() => run('archive', [cur])}>Archive</button>
        <button class="abtn" onclick={() => run('task', [cur])}>Task</button>
        <button class="abtn" onclick={() => ask('reply', 'Steering for the draft (optional)', 'decline politely; ask for the deadline', cur)}>Reply…</button>
        <button class="abtn" onclick={() => ask('needs-info', 'What are you waiting for?', 'their dates, before I can book', cur, { required: true })}>Park…</button>
      </div>
      <button class="morebtn" onclick={() => (more = !more)}>{more ? 'fewer actions' : 'more actions'}</button>
      <div class="barnote">Drafts land in the outbox for review — nothing sends from here.</div>
    </div>
  {:else if screen === 'sweep'}
    <header class="threadhead">
      <button class="iconbtn" onclick={() => (screen = 'list')} aria-label="Back to the list">{@render chevron('left')}</button>
      <span class="title">Sweep</span>
    </header>
    <div class="lanes sweeptabs" role="tablist" aria-label="Suggested action">
      {#each SWEEP_VERBS as v}
        <button class="lanechip" class:on={sweepVerb === v} role="tab" aria-selected={sweepVerb === v} onclick={() => setSweepVerb(v)}>
          {SWEEP_LABEL[v]} <span class="n">{sweepCounts[v]}</span>
        </button>
      {/each}
    </div>
    <div class="scroll">
      <h1 class="sweeptitle">{SWEEP_LABEL[sweepVerb]} {sweepCounts[sweepVerb]} thread{sweepCounts[sweepVerb] === 1 ? '' : 's'}?</h1>
      <p class="sweepnote">The classifier suggested this for each one. Untick a group to keep it in the queue; one Undo brings the whole batch back.</p>
      {#each groups as g (g.key)}
        <label class="group">
          <input type="checkbox" checked={ticked.has(g.key)} onchange={() => toggleGroup(g)} />
          <span class="grow groupbody">
            <strong>{g.name}</strong>
            {#if g.name !== g.key}<span class="addr">{g.key}</span>{/if}
            <span>{g.rows[0].subject || g.rows[0].summary}{g.rows.length > 1 ? ` + ${g.rows.length - 1} more` : ''}</span>
          </span>
          <span class="n">{g.rows.length}</span>
        </label>
      {:else}
        <div class="empty">Nothing to sweep for this action.</div>
      {/each}
    </div>
    <div class="actions">
      <button class="bigaccept" disabled={!checkedRows.length} onclick={applySweep}>{SWEEP_LABEL[sweepVerb]} {checkedRows.length} thread{checkedRows.length === 1 ? '' : 's'}</button>
    </div>
  {/if}

  {#if composing}
    <div class="cscrim" onclick={() => (composing = false)} aria-hidden="true"></div>
    <div class="sheet">
      <div class="sheet-grip"></div>
      <div class="sheet-text">New email — stages for review, never sends from here</div>
      <input class="editline" bind:value={cTo} placeholder="to: who@example.edu" />
      <input class="editline" bind:value={cSubject} placeholder="subject" />
      <input class="editline" bind:value={cAccount} placeholder="from account (blank = default)" />
      <textarea class="editbox" rows="5" bind:value={cBody} placeholder="the email, in your words (markdown ok)"></textarea>
      <div class="btnrow">
        <button class="btn" onclick={() => (composing = false)}>Back</button>
        <button class="btn primary" disabled={busy || !cTo.trim() || !cSubject.trim() || !cBody.trim()} onclick={stageCompose}>Stage it</button>
      </div>
    </div>
  {/if}

  {#if confirmSpam}
    <div class="cscrim" onclick={() => (confirmSpam = null)} aria-hidden="true"></div>
    <div class="sheet">
      <div class="sheet-grip"></div>
      <div class="warnline">{@render hazardGlyph()}<span>Spam trains the provider's filter — the one triage action with an effect outside your mailbox.</span></div>
      <div class="sheet-sub">{senderOf(confirmSpam)}{#if confirmSpam.from_name && confirmSpam.from} &lt;{confirmSpam.from}&gt;{/if} · {confirmSpam.subject || confirmSpam.summary}</div>
      <div class="btnrow">
        <button class="btn" onclick={() => (confirmSpam = null)}>Back</button>
        <button class="btn primary" onclick={markSpam}>Mark spam</button>
      </div>
    </div>
  {/if}

  {#if asking}
    <div class="cscrim" onclick={() => (asking = null)} aria-hidden="true"></div>
    <div class="sheet">
      <div class="sheet-grip"></div>
      <div class="sheet-text">{asking.label}</div>
      <div class="sheet-sub">{asking.rows[0].subject || asking.rows[0].summary}</div>
      {#if asking.wantTo}
        <input class="editline" bind:value={askTo} placeholder="who@example.edu, other@example.com" />
      {/if}
      <textarea class="editbox" rows="3" bind:value={askText} placeholder={asking.placeholder}></textarea>
      <div class="btnrow">
        <button class="btn" onclick={() => (asking = null)}>Back</button>
        <button
          class="btn primary"
          disabled={(asking.wantTo && !askTo.trim()) || (asking.required && !askText.trim())}
          onclick={submitAsk}
        >{asking.verb === 'needs-info' ? 'Park it' : 'Go'}</button>
      </div>
    </div>
  {/if}

  {#if toast}
    <!-- On the list it sits above the compose button; on the thread and sweep
         screens it goes under the header, clear of the Accept button, which is
         the next thing a thumb reaches for. -->
    <div class="toast" class:top={screen !== 'list'}>
      <span class="toasttext">{toast.text}</span>
      {#if toast.undo}<button class="undo" onclick={() => q.undo()}>Undo</button>{/if}
    </div>
  {/if}
</div>

<style>
  .page { flex: 1; display: flex; flex-direction: column; min-height: 0; position: relative; }
  .grow { flex: 1; min-width: 0; }
  .n { font-family: var(--mono); font-size: 11px; color: var(--text-muted); }
  .empty { color: var(--text-muted); font-size: 14px; padding: 24px 0; text-align: center; }
  .warnline { display: flex; align-items: flex-start; gap: 8px; font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .chip { font-family: var(--mono); font-size: 10px; padding: 1px 6px; border: 1px solid var(--accent-900); border-radius: 4px; color: var(--text-muted); white-space: nowrap; }
  .chip.due { border-color: transparent; background: #3a2e1a; color: var(--hazard); }
  .chip.bad { border-color: var(--hazard); color: var(--hazard); }
  .kicker { font-family: var(--mono); font-size: 11px; letter-spacing: 0.06em; text-transform: uppercase; color: var(--accent-500); }

  .listhead { padding: 14px var(--gutter-gear) 10px var(--gutter); border-bottom: 1px solid var(--accent-900); display: flex; flex-direction: column; gap: 10px; }
  .titlerow { display: flex; align-items: baseline; gap: 10px; }
  .title { font-weight: 500; font-size: 17px; letter-spacing: -0.02em; }
  .left { font-size: 12px; color: var(--text-muted); }
  .lanes { display: flex; gap: 6px; overflow-x: auto; margin: 0 calc(-1 * var(--gutter-gear)) 0 0; padding-right: var(--gutter); -webkit-overflow-scrolling: touch; }
  .lanes::-webkit-scrollbar { display: none; }
  .sweeptabs { padding: 10px var(--gutter); margin: 0; border-bottom: 1px solid var(--accent-900); }
  .lanechip { flex-shrink: 0; min-height: 38px; padding: 0 12px; border: 1px solid var(--accent-900); border-radius: var(--radius-chip); background: var(--bg); color: var(--text-muted); font-size: 13px; cursor: pointer; white-space: nowrap; }
  .lanechip.on { color: var(--text); background: var(--accent-900); border-color: var(--accent-700); }

  .scroll { flex: 1; overflow-y: auto; padding: 12px var(--gutter) 90px; display: flex; flex-direction: column; gap: 10px; }
  /* A flex item with overflow!=visible has automatic min-size ZERO, so a
     long list's cards silently shrink to fit the viewport and clip their
     own text — 24 rows rendered as 30px slivers on a real phone. Never
     let the scroll container's children shrink; scrolling is its job. */
  .scroll > * { flex-shrink: 0; }

  .sweepcard { display: flex; align-items: center; gap: 12px; min-height: 56px; padding: 10px 14px; border: 1px solid var(--accent-700); border-radius: var(--radius); background: var(--accent-900); text-align: left; cursor: pointer; color: var(--text); font: inherit; }
  .sweepcard strong { display: block; font-size: 14px; font-weight: 600; color: var(--accent-300); }
  .sweepcard span span, .sweepcard .grow span { font-size: 12px; color: var(--text-muted); }
  .sweepgo { font-size: 13px; font-weight: 600; color: var(--accent-400); }

  .rows { display: flex; flex-direction: column; margin: 0 calc(-1 * var(--gutter)); }
  .swipe { position: relative; overflow: hidden; border-bottom: 1px solid var(--accent-900); }
  .under { position: absolute; inset: 0; display: flex; align-items: center; padding: 0 20px; font-size: 13px; font-weight: 600; }
  .under.accepting { background: var(--accent-900); color: var(--accent-300); }
  .under.archiving { background: var(--surface); color: var(--text); }
  .row { position: relative; width: 100%; min-height: 80px; display: flex; gap: 12px; align-items: stretch; box-sizing: border-box; padding: 12px var(--gutter) 12px 0; border: 0; background: var(--void); color: var(--text); font: inherit; text-align: left; cursor: pointer; touch-action: pan-y; transition: transform 160ms ease; user-select: none; }
  .row.dragging { transition: none; }
  .urg { width: 3px; flex-shrink: 0; border-radius: 0 2px 2px 0; }
  .u-now { background: #d4526e; }
  .u-today { background: var(--hazard); }
  .u-week { background: var(--accent-500); }
  .rowbody { display: flex; flex-direction: column; gap: 3px; flex: 1; min-width: 0; }
  .line1, .line3 { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .from { font-size: 15px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .age { font-family: var(--mono); font-size: 11px; color: var(--text-muted); white-space: nowrap; }
  .subject { font-size: 14px; color: #c9c9d3; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .summary { flex: 1; min-width: 0; font-size: 13px; color: var(--text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .prop { flex-shrink: 0; font-family: var(--mono); font-size: 11px; padding: 2px 7px; border-radius: 4px; border: 1px solid var(--accent-900); color: var(--text-muted); }
  .p-reply, .p-forward { color: var(--accent-400); }
  .p-task { color: #e0a458; }
  .p-schedule { color: #7fc4b8; }
  .p-spam { color: #d4526e; }
  .unread { width: 7px; height: 7px; border-radius: 50%; background: var(--accent-400); flex-shrink: 0; }
  .hint { position: absolute; left: 0; right: 0; bottom: 26px; text-align: center; font-size: 11px; color: var(--text-muted); pointer-events: none; }

  .threadhead { display: flex; align-items: center; gap: 4px; padding: 6px var(--gutter-gear) 6px 6px; border-bottom: 1px solid var(--accent-900); min-height: 52px; box-sizing: border-box; }
  .threadhead .title { margin-left: 4px; }
  .iconbtn { width: 44px; height: 44px; display: flex; align-items: center; justify-content: center; background: none; border: none; color: var(--text); cursor: pointer; }
  .iconbtn:disabled { opacity: 0.3; cursor: default; }
  .pos { font-size: 13px; color: var(--text-muted); }
  .reader { padding-bottom: 16px; gap: 14px; }
  .reader h1, .sweeptitle { margin: 0; font-size: 19px; font-weight: 600; line-height: 1.3; overflow-wrap: anywhere; }
  .meta { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; font-size: 12px; color: var(--text-muted); }
  .meta .who { color: #c9c9d3; }
  .addr { font-family: var(--mono); font-size: 11px; color: var(--text-muted); overflow-wrap: anywhere; }
  .suggest { display: flex; flex-direction: column; gap: 4px; padding: 12px 14px; border: 1px solid var(--accent-700); border-radius: var(--radius); background: var(--accent-900); font-size: 14px; line-height: 1.45; }
  .suggest strong { color: var(--accent-300); font-weight: 600; }
  .msg { display: flex; flex-direction: column; gap: 8px; }
  .msg-meta { font-family: var(--mono); font-size: 11px; color: var(--text-muted); overflow-wrap: anywhere; }
  .msg-subject { font-size: 15px; font-weight: 500; line-height: 1.4; overflow-wrap: anywhere; }
  .quoted { display: flex; gap: 10px; }
  .gutter { width: 2px; background: var(--hazard); flex-shrink: 0; }
  .mailbody { font-size: 15px; line-height: 1.6; color: var(--text); white-space: pre-wrap; overflow-wrap: anywhere; }
  .qtext { font-size: 13px; line-height: 1.55; color: var(--text-muted); white-space: pre-wrap; overflow-wrap: anywhere; }

  .actions { flex-shrink: 0; display: flex; flex-direction: column; gap: 8px; padding: 10px var(--gutter) calc(6px + env(safe-area-inset-bottom)); border-top: 1px solid var(--accent-900); background: var(--bg); }
  .bigaccept { min-height: 52px; border: none; border-radius: 12px; background: var(--accent-400); color: var(--void); font-size: 16px; font-weight: 600; cursor: pointer; }
  .bigaccept:disabled { opacity: 0.5; cursor: default; }
  .grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 8px; }
  .abtn { min-height: 46px; padding: 0 4px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 13px; cursor: pointer; }
  .morebtn { background: none; border: none; color: var(--text-muted); font-size: 12px; min-height: 32px; cursor: pointer; }
  .barnote { font-size: 10px; color: var(--text-muted); text-align: center; }

  .sweepnote { margin: 0; font-size: 13px; line-height: 1.5; color: var(--text-muted); }
  .group { display: flex; align-items: center; gap: 12px; min-height: 60px; box-sizing: border-box; padding: 10px 14px; border: 1px solid var(--accent-900); border-radius: var(--radius); background: var(--bg); cursor: pointer; }
  .group input { width: 20px; height: 20px; accent-color: var(--accent-500); flex-shrink: 0; }
  .groupbody { display: flex; flex-direction: column; gap: 2px; }
  .groupbody strong { font-size: 15px; font-weight: 600; }
  .groupbody span { font-size: 12px; color: var(--text-muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  .btnrow { display: flex; gap: 10px; }
  .btn { flex: 1; min-height: 48px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; }
  .btn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .editbox { width: 100%; background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; line-height: 1.5; padding: 12px 14px; resize: vertical; box-sizing: border-box; }
  .editline { width: 100%; background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius); color: var(--text); font-family: var(--mono); font-size: 13px; padding: 12px 14px; box-sizing: border-box; }
  /* Above the scrim (5) and the fab (4), matching Tasks/Frontdoor — at
     z auto the scrim paints over the sheet and swallows every tap. */
  .sheet { position: absolute; left: 0; right: 0; bottom: 0; background: var(--bg); border-top: 1px solid var(--accent-500); border-radius: 16px 16px 0 0; padding: 14px var(--gutter) 28px; display: flex; flex-direction: column; gap: 12px; z-index: 6; }
  .sheet-grip { width: 36px; height: 4px; border-radius: 2px; background: var(--accent-900); align-self: center; }
  .sheet-text { font-size: 15px; font-weight: 500; }
  .sheet-sub { font-size: 13px; color: var(--text-muted); overflow-wrap: anywhere; }
  .fab { position: absolute; right: 18px; bottom: 18px; width: 54px; height: 54px; border-radius: 50%; background: var(--accent-400); border: none; cursor: pointer; display: flex; align-items: center; justify-content: center; box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4); z-index: 4; }
  .cscrim { position: absolute; inset: 0; background: rgba(0, 0, 0, 0.45); z-index: 5; }
  .toast { position: absolute; left: 16px; right: 16px; bottom: 84px; z-index: 7; display: flex; align-items: center; gap: 10px; min-height: 48px; box-sizing: border-box; padding: 6px 6px 6px 14px; background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius); box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4); font-size: 13px; }
  .toast.top { top: 60px; bottom: auto; }
  .toasttext { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .undo { min-height: 36px; padding: 0 12px; background: none; border: none; color: var(--accent-400); font-size: 14px; font-weight: 600; cursor: pointer; }
</style>
