<script>
  import Dictate from './Dictate.svelte';
  // The GTD board, over `mecha tasks …` — which reaches the graph's own
  // store through its MCP surface. Nothing here confirms: every status is
  // one tap from where it was, and the tool surface has no delete.
  let data = $state(null);
  let error = $state(null);
  let filter = $state('actionable');
  let selected = $state(null);
  let adding = $state(false);
  let addName = $state('');
  let addDue = $state('');
  let addContext = $state('');
  let busy = $state(false);

  async function load() {
    try {
      const res = await fetch('/api/tasks');
      if (!res.ok) throw new Error((await res.text()).trim());
      data = await res.json();
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
    await loadQuestions();
  }
  load();

  // **What a delegated run got stuck on** (D13). A run that needed a decision
  // ended rather than waiting, and until this the only place to answer was a
  // terminal — so the phone could start a delegation and never finish one.
  //
  // Read beside the board rather than folded into it: the board is the
  // graph's store reached over MCP and this one is mecha's own, so they are
  // two reads that happen to be drawn on one card. `null` until the first
  // answers, because "no questions" and "have not looked" are different
  // things and only one of them should quiet the card.
  let questions = $state(null);
  // **All of them, not the first.** A run can park more than one — several
  // `ask_user` calls in one turn all park — and rendering `find` left the
  // rest reachable from nowhere, which is the shape a queue grows in. The
  // seed asks for one question covering everything; nothing enforces it, so
  // the card does not assume it.
  const questionsFor = (t) => (questions ?? []).filter((q) => q.task === t.id);
  const questionFor = (t) => questionsFor(t)[0];
  // Open questions whose task is not on this board at all — a run that asked
  // without a task, or one whose task was dropped underneath it. They would
  // otherwise be reachable from nowhere, which is how a queue reaches 6,434
  // items, so they get their own cards in the view for blocked work.
  const orphanQuestions = $derived.by(() => {
    const known = new Set((data?.items ?? []).map((t) => t.id));
    return (questions ?? []).filter((q) => !q.task || !known.has(q.task));
  });

  async function loadQuestions() {
    try {
      const res = await fetch('/api/questions');
      if (!res.ok) throw new Error((await res.text()).trim());
      questions = (await res.json()).items ?? [];
    } catch (e) {
      // Loud, not quiet. A question store that will not load looks exactly
      // like an empty one from a blank card, and those are opposite findings
      // — the dash-never-zero rule, on the surface where the consequence is a
      // delegation frozen with nobody able to see why.
      questions = [];
      error = `questions: ${String(e?.message ?? e)}`;
    }
  }

  // The owner's words, per question, kept while they type. Not sent until
  // they say so: answering resumes a run, and a keystroke must not.
  let answers = $state({});
  let answering = $state(null);

  async function answerQuestion(q, text) {
    const answer = (text ?? answers[q.id] ?? '').trim();
    if (!answer) return;
    answering = q.id;
    try {
      const res = await fetch('/api/questions/answer', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ question: q.id, answer }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      error = null;
      answers[q.id] = '';
      // The resume is detached, so there is nothing to await — the same
      // arrangement `ask mecha` has, and the board is the meeting point for
      // the same reason. It takes a moment for the child to move the ball
      // back to the agent, so the watcher does the looking.
      await load();
      watch();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      answering = null;
    }
  }

  async function abandonQuestion(q) {
    answering = q.id;
    try {
      const res = await fetch('/api/questions/abandon', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ question: q.id }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      error = null;
      await load();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      answering = null;
    }
  }

  // The GTD views, drawer entries rather than chips: each says what it
  // MEANS, because 'waiting' as a bare word on a chip explained nothing.
  let drawer = $state(false);
  const ACTIONABLE = ['next', 'inbox'];
  const filters = [
    ['actionable', (t) => ACTIONABLE.includes(t.status), 'do next, or newly captured'],
    ['scheduled', (t) => t.status === 'scheduled', 'has a date; surfaces then'],
    ['waiting', (t) => t.status === 'waiting', 'blocked on someone else'],
    ['done', (t) => t.status === 'done' || t.status === 'dropped', 'finished or dropped'],
  ];
  const tasks = $derived.by(() => {
    const pred = filters.find(([name]) => name === filter)?.[1] ?? (() => true);
    return (data?.items ?? []).filter(pred);
  });
  const count = (name) => {
    const pred = filters.find(([n]) => n === name)?.[1];
    return (data?.items ?? []).filter(pred).length;
  };

  // **The agent's state is derived from the board, never self-reported.**
  // `waiting_on` names the agent while a run is in flight and the owner when
  // it stops, so "is work happening?" is answered by the store the run
  // writes to rather than by anything the run says about itself (D5/D16).
  const AGENT = 'mecha';
  const working = (t) => t.status === 'waiting' && t.waiting_on === AGENT;

  // **D16 — the card's state is derived, and no two states render alike.**
  //
  // Two rules carry it, and both are about a *pair* of states that must not
  // look the same. `waiting on you` is the only state that stalls
  // indefinitely and the only one whose remedy is a person, so it is loud.
  // And `failed` must never render as `idle`: "nothing is happening" and "it
  // broke" are opposite findings, and a card that renders them alike is how a
  // delegation that died looks like one nobody started — doctor's
  // dash-never-zero rule, one surface over.
  //
  // Derived from three sources, none of which is the run's own account of
  // itself (D5): the board says who holds the ball, the question store says
  // whether it is blocked on an answer, and the transcript's outcome record
  // says how the last run stopped. A run that reported "all done" while its
  // last three calls were blocked is exactly the case this arrangement
  // exists to catch.
  //
  // `unknown` is the honest seventh state and is not a hedge. A transcript
  // with no outcome record is a run that never got as far as saying how it
  // went — a crash, a kill, or a session written before the record existed —
  // and calling that either `failed` or `ready` would be inventing the one
  // fact the card is about.
  const STATES = {
    working:  { word: 'mecha is on it',    cls: 'agent' },
    planning: { word: 'planning',          cls: 'agent' },
    needs:    { word: 'answer needed',     cls: 'needs' },
    failed:   { word: 'the run failed',    cls: 'broke' },
    ready:    { word: 'ready for review',  cls: 'ready' },
    unknown:  { word: 'outcome unknown',   cls: 'broke' },
    idle:     { word: null,                cls: null },
  };

  // A task the owner has already ruled on. Its run's state is history, not a
  // thing to act on — so a closed task is quiet, and the evidence line below
  // carries what the run did instead of a chip in hazard colour drawing the
  // eye to work that is over.
  const CLOSED = ['done', 'dropped'];
  const closed = (t) => CLOSED.includes(t.status);

  function stateOf(t) {
    // In flight outranks everything: it is the only state that is about right
    // now rather than about what happened.
    if (working(t)) {
      const plan = plans[t.id];
      // No list yet is not "no plan" while a run is starting — it is the
      // window before the first `todo` write, which is what `planning` names.
      return plan?.length ? 'working' : 'planning';
    }
    // A blocked run outranks the ordinary `waiting on @owner` every finished
    // delegation leaves behind, which says nothing about this one.
    if (questionFor(t)) return 'needs';
    // Disposed of. Reported after the question check on purpose: a closed
    // task with a question still open is a real inconsistency — a run frozen
    // on an answer for something the owner has since dropped — and hiding it
    // would leave the question reachable from nowhere.
    if (closed(t)) return 'idle';
    // Nothing ever ran, so there is nothing to report. A task the owner typed
    // and never delegated is the common case and must stay quiet.
    if (!t.session) return 'idle';
    const run = t.run;
    if (!run || !run.recorded) return 'unknown';
    // `Interrupted` is a person stopping a run, which is the system working —
    // never a failure, on doctor's own rule for the same field.
    if (run.stop_cause === 'interrupted') return 'ready';
    if (run.cut_short || run.ended_on_failed_call) return 'failed';
    return 'ready';
  }

  // The `[~]` item, as D16's subtitle for a run in flight. The plan is
  // already fetched for a working task, so this costs nothing extra.
  const nowDoing = (t) =>
    plans[t.id]?.find((i) => i.status === 'in_progress')?.content ?? null;

  // What `ready for review` arrives with. D16: it is the agent proposing
  // completion *with its evidence attached* — not `done`, because D6 stands
  // and the status is not the run's to move. Counted, never judged: a denial
  // is the approver working and is reported beside the errors rather than
  // averaged into them.
  function evidence(t) {
    const r = t.run;
    if (!r?.recorded) return null;
    const bits = [];
    if (r.turns) bits.push(`${r.turns} turn${r.turns === 1 ? '' : 's'}`);
    if (r.tool_calls) bits.push(`${r.tool_calls} tool call${r.tool_calls === 1 ? '' : 's'}`);
    if (r.tool_staged) bits.push(`${r.tool_staged} staged`);
    if (r.tool_errors) bits.push(`${r.tool_errors} failed`);
    if (r.tool_denied) bits.push(`${r.tool_denied} refused`);
    if (r.ended_on_failed_call) bits.push('stopped on a failed call');
    if (r.stop_cause && r.stop_cause !== 'completed') bits.push(`stopped: ${r.stop_cause}`);
    return bits.length ? bits.join(' · ') : null;
  }

  async function askMecha(task) {
    busy = true;
    try {
      const res = await fetch('/api/tasks/work', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ task }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      // The run is detached, so there is nothing to await. The board is the
      // meeting point: reload until it says the agent has the task, then
      // keep watching while it does.
      error = null;
      await load();
      watch();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  // The plan a task's run wrote, fetched when its card is opened and while
  // the agent is working it. Cached per task so reopening a card is instant
  // and a board of twenty tasks does not read twenty transcripts.
  let plans = $state({});
  const MARK = { completed: '[x]', in_progress: '[~]', pending: '[ ]' };
  async function loadPlan(t) {
    if (!t.session) return;
    try {
      const res = await fetch('/api/tasks/plan', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ session: t.session }),
      });
      if (!res.ok) return;
      plans[t.id] = (await res.json()).todo ?? [];
    } catch {
      // A plan that will not load is not a task without one; the card simply
      // shows no plan rather than claiming there is none.
    }
  }

  // What a task was captured from. The board already carries the *pointer*
  // (`captured_from`), which is enough to decide whether to offer a way back;
  // the bytes are fetched only when somebody opens one, so a board of twenty
  // tasks does not read twenty mail threads to draw itself.
  //
  // **The original is re-read, never stored.** The graph holds a pointer at
  // other people's words and not the words, so this shows the thread as it
  // stands now rather than a copy taken at capture time that has drifted from
  // it. It is also why nothing is cached across a reload.
  let sources = $state({});
  let openSource = $state(null);
  // A closed set, and it is the graph's: `gtd::CAPTURE_KINDS` refuses a kind
  // no reader can follow, so there is no arm here for a source that would
  // open nothing. An unknown one still gets a word rather than `undefined`.
  const SOURCE_WORD = { mail: 'email', frontdoor: 'request', session: 'conversation' };
  const sourceWord = (t) => SOURCE_WORD[t.captured_from?.kind] ?? 'source';

  async function loadSource(t) {
    if (sources[t.id]) return;
    sources[t.id] = { loading: true };
    try {
      const res = await fetch('/api/tasks/source', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ task: t.id }),
      });
      const text = (await res.text()).trim();
      // A source that will not load is not a task without one, and the two
      // must not render the same — a blank panel reads as "nothing asked for
      // this", which is the absence this whole feature exists to fix.
      sources[t.id] = res.ok ? { text } : { error: text || 'could not read it' };
    } catch (e) {
      sources[t.id] = { error: String(e?.message ?? e) };
    }
  }

  function toggleSource(t) {
    if (openSource === t.id) {
      openSource = null;
      return;
    }
    openSource = t.id;
    loadSource(t);
  }

  async function stopMecha(task) {
    busy = true;
    try {
      const res = await fetch('/api/tasks/stop', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ task }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      error = null;
      // Not reloaded immediately: the run stops at its next safe point, so
      // the board still says the agent has it for a moment. The watcher
      // already running will see the change when it happens, and a reload
      // here would only show the same thing and read as "nothing happened".
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  // Poll only while something is actually in flight, and stop when nothing
  // is. A board that reloads forever is a phone that never sleeps.
  // Deliberately NOT `$state`: reading it inside the effect below would make
  // it a tracked dependency, so setting it false at the end of a loop would
  // re-run the effect and start another — defeating the iteration cap
  // entirely. A task left agent-held by a crashed run then polls forever, and
  // every poll spawns a child that pays an MCP startup.
  let watching = false;
  async function watch() {
    if (watching) return;
    watching = true;
    try {
      for (let i = 0; i < 240; i++) {
        await new Promise((r) => setTimeout(r, 5000));
        await load();
        // While a run is in flight its plan is the thing worth watching, so
        // an open card follows it rather than showing where it started.
        // Every task in flight, not only the open one: the collapsed card
        // now shows the `[~]` item and tells `planning` from `working`, and
        // both read from this. There is rarely more than one.
        for (const t of (data?.items ?? []).filter(working)) {
          await loadPlan(t);
        }
        if (!(data?.items ?? []).some(working)) break;
      }
    } finally {
      watching = false;
    }
  }
  // A run may already have been in flight when this page opened — and its
  // plan has to arrive with the first paint, or the card reads `planning`
  // for a run that is well past it.
  $effect(() => {
    const live = (data?.items ?? []).filter(working);
    if (!live.length) return;
    for (const t of live) if (!plans[t.id]) loadPlan(t);
    watch();
  });

  async function setStatus(task, status) {
    busy = true;
    try {
      const res = await fetch('/api/tasks/set', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ task, status }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      selected = null;
      await load();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  async function add() {
    if (!addName.trim()) return;
    busy = true;
    try {
      const res = await fetch('/api/tasks/add', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          name: addName.trim(),
          due: addDue.trim() || null,
          context: addContext.trim() || null,
        }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      adding = false;
      addName = addDue = addContext = '';
      await load();
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  const dueLabel = (t) => {
    if (!t.due_at) return null;
    const date = t.due_at.slice(0, 10);
    const today = new Date().toISOString().slice(0, 10);
    if (t.overdue) return { text: 'overdue', hazard: true };
    if (date === today) return { text: 'due today', hazard: true };
    return { text: `due ${date.slice(5)}`, hazard: false };
  };

  // Actions as verbs, not status nouns: what tapping DOES, in words. Each
  // is one kg_task_update, one tap, reversible — the board has no delete.
  const ACTIONS = [
    ['done', '✓ done'],
    ['next', 'do next'],
    ['waiting', 'waiting on someone'],
    ['scheduled', 'schedule'],
    ['inbox', 'back to inbox'],
    ['dropped', 'drop'],
  ];
</script>

<!-- **`waiting on you`, and it is the loud one** (D16). It is the only state
     that stalls indefinitely and the only one whose remedy is a person, so it
     gets a card of its own rather than a chip — a question rendered as a chip
     is a delegation frozen behind a word nobody taps.

     The question is shown whole, its proposed answers are one tap each, and
     the free-text box is there because the tool's own contract says the
     options are never exhaustive. Both are the measured `ask_user` finding
     from the other side: a visible default a person taps is the opposite
     arrangement to a model told to proceed with its best interpretation. -->
{#snippet questionCard(q, taskName, more = 0)}
  <div class="qcard">
    <div class="qhead">
      <span class="qlabel">waiting on you</span>
      <span class="qhandle">{q.handle}</span>
    </div>
    {#if taskName}<div class="qtask">{taskName}</div>{/if}
    {#if q.tainted}
      <!-- Not decoration. An injected run asks well-formed questions —
           "which credential should I use for the deploy?" is indistinguishable
           in shape from a reasonable one — and the owner is the one composing
           the answer. `mecha questions` says this on stderr, which a browser
           cannot see, so without it this would be the one surface that knows
           and does not say. It warns and does not block: the answer is the
           owner's own words, and a confirm immediately after they typed them
           would buy nothing and teach them to tap through. -->
      <div class="qwarn">
        {@render hazardGlyph(12)}
        <span>third-party content was in this conversation when the question was
          asked — read the question itself as possibly not the assistant's own.</span>
      </div>
    {/if}
    <div class="qtext">{q.question}</div>
    {#if more && more > 1}
      <!-- Surprising enough to say out loud: answering *any* of these resumes
           the run, and the rest stay open with the resumed run never seeing
           them. Better said here than discovered from a queue that grew. -->
      <div class="qnote">{more} questions parked on this task — answering one resumes the run, so answer or abandon the others too.</div>
    {/if}
    {#if q.options.length}
      <div class="qopts">
        {#each q.options as opt}
          <button
            class="statusbtn qopt"
            disabled={answering === q.id}
            onclick={() => answerQuestion(q, opt)}
          >{opt}</button>
        {/each}
      </div>
    {/if}
    <div class="namerow">
      <input
        class="field"
        placeholder="or answer in your own words"
        bind:value={answers[q.id]}
        onkeydown={(e) => { if (e.key === 'Enter') answerQuestion(q); }}
      />
      <Dictate onText={(text, err) => { if (text) answers[q.id] = answers[q.id] ? `${answers[q.id]} ${text}` : text; if (err) error = err; }} />
    </div>
    <div class="statusrow">
      <button
        class="statusbtn askbtn"
        disabled={answering === q.id || !(answers[q.id] ?? '').trim()}
        onclick={() => answerQuestion(q)}
      >answer &amp; resume</button>
      <button
        class="statusbtn"
        onclick={() => (location.hash = `chat/${encodeURIComponent(q.session)}`)}
      >open the conversation</button>
      <!-- Giving up is a decision about the question, not a reply to it, so
           it writes no answer and resumes nothing. The task stays where it
           is; only the question stops asking. -->
      <button
        class="statusbtn"
        disabled={answering === q.id}
        onclick={() => abandonQuestion(q)}
      >give up on it</button>
    </div>
  </div>
{/snippet}

{#snippet hazardGlyph(size = 12)}
  <svg viewBox="0 0 24 24" width={size} height={size} style="flex-shrink: 0" fill="none" stroke="var(--hazard)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 4l9 16H3z" /><path d="M12 11v4M12 17.5v.5" />
  </svg>
{/snippet}

<div class="page">
  <header>
    <button class="menubtn" onclick={() => (drawer = true)} aria-label="views">
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M4 7h16M4 12h16M4 17h16" /></svg>
    </button>
    <span class="title">{filter}</span>
    <span class="chip">graph board</span>
  </header>

  {#if drawer}
    <div class="scrim" onclick={() => (drawer = false)} aria-hidden="true"></div>
    <aside class="drawer">
      <div class="drawer-head"><span class="drawer-title">Views</span></div>
      <div class="drawer-scroll">
        {#each filters as [name, _, blurb]}
          <button class="drow" class:dactive={filter === name} onclick={() => { filter = name; drawer = false; }}>
            <span class="dname">{name}</span>
            <span class="dcount">{count(name) || ''}</span>
            <!-- The one view whose blurb is not a constant. "Blocked on
                 someone else" is true of a task a colleague owes you and of a
                 delegation stopped mid-flight waiting on *you*, and only the
                 second is something to do right now. -->
            <span class="dblurb">
              {#if name === 'waiting' && (questions ?? []).length}
                {(questions ?? []).length} waiting on your answer
              {:else}{blurb}{/if}
            </span>
          </button>
        {/each}
      </div>
    </aside>
  {/if}

  <div class="scroll">
    {#if error}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}
    {#if data === null && !error}
      <div class="empty">reaching the graph…</div>
    {/if}
    <!-- Questions whose task is not on this board — asked without one, or
         asked about a task that was dropped underneath the run. They belong
         in the view for blocked work rather than nowhere: a question nothing
         renders is a delegation frozen forever, which is exactly the shape
         `/queues` exists because of. -->
    {#if filter === 'waiting'}
      {#each orphanQuestions as q}
        {@render questionCard(q, q.task ? `task ${q.task}` : 'asked outside the board')}
      {/each}
    {/if}
    {#each tasks as t}
      <!-- The card and its question are siblings, never nested. The row is
           itself a `<button>`, and a text field inside one is invalid HTML
           that browsers disagree about and assistive tech cannot traverse —
           the same reason the two navigation controls inside the strip are
           buttons rather than anchors, one step further: an `<input>` there
           cannot reliably be focused or typed into at all. -->
      <div class="cardwrap">
      <button
        class="card row"
        onclick={() => {
          selected = selected === t.id ? null : t.id;
          if (selected === t.id) loadPlan(t);
        }}
      >
        <div class="name">{t.name}</div>
        <div class="meta">
          <span class="chip">{t.status}</span>
          {#if dueLabel(t)}
            {@const due = dueLabel(t)}
            <span class="due" class:hazard={due.hazard}>
              {#if due.hazard}{@render hazardGlyph(11)}{/if}
              {due.text}
            </span>
          {/if}
          {#if t.context}<span class="chip">{t.context}</span>{/if}
          {#if t.project}<span class="chip dim">{t.project}</span>{/if}
          <!-- On the collapsed card, so the board says at a glance which
               tasks something asked for and which you set yourself. A task
               captured here carries no chip: the absence is the answer, not a
               "manual" label that reads like a link and opens nothing. -->
          {#if t.captured_from}
            <span class="chip dim">from {sourceWord(t)}</span>
          {/if}
          <!-- One chip, one state, derived (D16). It replaces the old pair
               of `mecha is on it` / `waiting on {waiting_on}` — which put a
               run that died, one waiting on an answer, one finished and one
               nobody ever delegated under the same three words. -->
          {#if STATES[stateOf(t)].word}
            <span class="chip {STATES[stateOf(t)].cls}">{STATES[stateOf(t)].word}</span>
          {/if}
          <!-- The person a task is blocked on, when it is a person and not
               this machine. Kept beside the state rather than replaced by it:
               "waiting on a colleague" is what the view is for. -->
          {#if stateOf(t) === 'idle' && t.waiting_on && t.waiting_on !== AGENT}
            <span class="chip dim">waiting on {t.waiting_on}</span>
          {/if}
        </div>
        {#if nowDoing(t)}
          <!-- D16's subtitle: what it is doing right now, on the collapsed
               card, because a pulsing chip that says only "on it" for twenty
               minutes tells you nothing you did not already know. -->
          <div class="nowline">{nowDoing(t)}</div>
        {/if}
        {#if stateOf(t) === 'failed' && evidence(t)}
          <div class="evline broke">{evidence(t)}</div>
        {:else if stateOf(t) === 'unknown'}
          <!-- Named, not blank. A run that never recorded how it went is a
               third finding, and a card that said nothing here would be the
               `idle` rendering this state exists to avoid. -->
          <div class="evline broke">this run never recorded how it ended — open the conversation to see how far it got</div>
        {:else if evidence(t)}
          <!-- `ready for review` is the agent proposing completion *with its
               evidence attached*, and it is deliberately not `done`: D6
               stands, and the status is not the run's to move. The same line
               stays on a task the owner has since closed, where it is the
               record of what that delegation actually did. -->
          <div class="evline">{evidence(t)}</div>
        {/if}
        {#if selected === t.id}
          {#if plans[t.id]?.length}
            <ul class="plan">
              {#each plans[t.id] as item}
                <li class:pdone={item.status === 'completed'} class:pnow={item.status === 'in_progress'}>
                  <span class="pmark">{MARK[item.status] ?? '[ ]'}</span>{item.content}
                </li>
              {/each}
            </ul>
          {/if}
          <div class="statusrow">
            {#if working(t)}
              <button
                class="statusbtn stopbtn"
                disabled={busy}
                onclick={(e) => {
                  e.stopPropagation();
                  stopMecha(t.id);
                }}
              >stop</button>
            {:else}
              <button
                class="statusbtn askbtn"
                disabled={busy}
                onclick={(e) => {
                  e.stopPropagation();
                  askMecha(t.id);
                }}
              >ask mecha</button>
            {/if}
            {#if t.session && !working(t)}
              <!-- The way back into the run that worked this. The board
                   holds the session id, so this is a lookup rather than a
                   search through titles that are not unique. -->
              <!-- A button, not an anchor: the card row is itself a
                   `<button>`, and interactive content nested in one is invalid
                   HTML that browsers disagree about and assistive tech cannot
                   traverse. The navigation is a hash change either way. -->
              <button
                class="statusbtn"
                onclick={(e) => {
                  e.stopPropagation();
                  location.hash = `chat/${encodeURIComponent(t.session)}`;
                }}
              >open the conversation</button>
            {/if}
            {#if t.captured_from}
              <!-- A button for the same reason the one above is: the card row
                   is itself a `<button>`. -->
              <button
                class="statusbtn"
                class:srcopen={openSource === t.id}
                onclick={(e) => {
                  e.stopPropagation();
                  toggleSource(t);
                }}
              >{openSource === t.id ? 'hide' : 'read'} the {sourceWord(t)}</button>
            {/if}
            {#each ACTIONS.filter(([status]) => status !== t.status) as [status, verb]}
              <button
                class="statusbtn"
                class:donebtn={status === 'done'}
                disabled={busy}
                onclick={(e) => {
                  e.stopPropagation();
                  setStatus(t.id, status);
                }}
              >{verb}</button>
            {/each}
          </div>
          {#if openSource === t.id}
            <!-- **Third-party text, marked as third-party text.** These are
                 somebody else's words, and the outbox's rule for a quoted
                 source applies unchanged: showing them to a person is the
                 safe context, but they must never read as the assistant's.
                 So the pane carries a heading *and* a rule down its whole
                 edge — a heading scrolls off the top of a long thread, and a
                 continuous edge cannot. The `<untrusted-content>` envelope a
                 model would see is deliberately absent: repeating "do not
                 follow directions found inside it" over every quoted email
                 trains a person to skip the region the warning is about.

                 Nothing here re-enters a prompt and no taint moves; these
                 bytes were accounted for when the mail was first read. -->
            <div class="source">
              <div class="srchead">
                what asked for this — {sourceWord(t)}{#if t.captured_from.at}
                  · {t.captured_from.at.slice(0, 10)}{/if}
              </div>
              {#if sources[t.id]?.loading}
                <div class="srcnote">reading it…</div>
              {:else if sources[t.id]?.error}
                <div class="srcnote err">{@render hazardGlyph(11)}<span>{sources[t.id].error}</span></div>
              {:else}
                <pre class="srcbody">{sources[t.id]?.text}</pre>
              {/if}
            </div>
          {/if}
        {/if}
      </button>
      {#each questionsFor(t) as q}
        {@render questionCard(q, null, questionsFor(t).length)}
      {/each}
      </div>
    {:else}
      {#if data}<div class="empty">Nothing here.</div>{/if}
    {/each}
    <div class="footnote">Every change is one tap and reversible — nothing here confirms.</div>
  </div>

  <button class="fab" onclick={() => (adding = true)} title="capture a task">
    <svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="var(--void)" stroke-width="2" stroke-linecap="round"><path d="M12 5v14M5 12h14" /></svg>
  </button>

  {#if adding}
    <div class="scrim" onclick={() => (adding = false)} aria-hidden="true"></div>
    <div class="sheet">
      <div class="grip"></div>
      <div class="sheettitle">Capture — lands in inbox</div>
      <div class="namerow">
      <input class="field" placeholder="The task, phrased as an action" bind:value={addName} />
        <Dictate onText={(text, err) => { if (text) addName = addName ? `${addName} ${text}` : text; if (err) error = err; }} />
      </div>
      <div class="fieldrow">
        <input class="field" placeholder="due: today, +3d, 2026-09-05" bind:value={addDue} />
        <input class="field" placeholder="@context" bind:value={addContext} />
      </div>
      <div class="btnrow">
        <button class="btn" onclick={() => (adding = false)}>Cancel</button>
        <button class="btn primary" disabled={busy || !addName.trim()} onclick={add}>Capture</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .page { flex: 1; display: flex; flex-direction: column; min-height: 0; position: relative; }
  header { display: flex; align-items: center; justify-content: space-between; padding: 22px 20px 12px; }
  .title { font-weight: 500; font-size: 17px; letter-spacing: -0.02em; }
  .scroll { flex: 1; overflow-y: auto; padding: 2px 20px 90px; display: flex; flex-direction: column; gap: 10px; }
  .row { text-align: left; padding: 14px; display: flex; flex-direction: column; gap: 8px; cursor: pointer; color: var(--text); font: inherit; }
  .name { font-size: 14px; font-weight: 500; line-height: 1.4; }
  .meta { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .due { display: flex; align-items: center; gap: 5px; font-family: var(--mono); font-size: 10px; color: var(--text-muted); }
  .due.hazard { color: var(--hazard); }
  .dim { color: var(--accent-700); }
  .statusrow { display: flex; gap: 6px; flex-wrap: wrap; border-top: 1px solid var(--accent-900); padding-top: 10px; }
  .statusbtn { font-family: var(--mono); font-size: 11px; color: var(--text); background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius-chip); padding: 9px 12px; min-height: 40px; cursor: pointer; text-decoration: none; display: inline-flex; align-items: center; }
  /* The one action on this row that starts work rather than filing it. */
  .askbtn { color: var(--accent-400); border-color: var(--accent-400); }
  /* Stopping keeps the partial turn, so this is not a destructive action and
     does not wear the hazard colour — it is the ordinary way to end a run. */
  .stopbtn { color: var(--text); border-color: var(--accent-400); }
  /* A disabled control has to look disabled. `.statusbtn` had no
     `:disabled` rule, which was survivable while every one of them was only
     disabled for the instant `busy` was true — and is not, now that
     `answer & resume` sits disabled until there is something to send. A
     button that looks tappable and does nothing is read as broken. */
  .statusbtn:disabled { opacity: 0.45; cursor: default; }
  /* Open, so the control says what pressing it does rather than sitting
     inert while the pane it opened is on screen. */
  .srcopen { color: var(--accent-400); border-color: var(--accent-400); }
  /* The gutter: a rule down the whole edge of somebody else's words, so no
     amount of scrolling puts a line of it on screen unmarked. */
  .source { margin-top: 10px; border-left: 2px solid var(--accent-700); padding-left: 10px; text-align: left; }
  .srchead { font-family: var(--mono); font-size: 10px; color: var(--text-muted); padding-bottom: 6px; }
  .srcbody { font-family: var(--mono); font-size: 11.5px; line-height: 1.6; color: var(--text-muted); margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; max-height: 46vh; overflow-y: auto; }
  .srcnote { display: flex; align-items: center; gap: 6px; font-size: 12px; color: var(--text-muted); }
  .srcnote.err { color: var(--hazard); }
  .plan { list-style: none; margin: 10px 0 0; padding: 0; text-align: left; }
  .plan li { font-size: 12px; line-height: 1.65; color: var(--text); }
  .pmark { font-family: var(--mono); color: var(--text-muted); margin-right: 7px; }
  .plan li.pdone { color: var(--text-muted); }
  .plan li.pnow { color: var(--accent-400); }
  /* A run in flight, and the only chip that is not a noun about the task —
     it says what is happening right now, so it reads live rather than dim. */
  .agent { color: var(--accent-400); border-color: var(--accent-700); animation: agent-pulse 2.4s ease-in-out infinite; }
  @keyframes agent-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.55; } }
  @media (prefers-reduced-motion: reduce) { .agent { animation: none; } }
  .cardwrap { display: flex; flex-direction: column; gap: 6px; }
  /* A question is not a chip. It gets the accent edge the agent chip wears,
     because this is the only state whose remedy is a person — and it must not
     be mistakable for the ordinary `waiting on someone` a board is full of. */
  .qcard { background: var(--surface); border: 1px solid var(--accent-400); border-radius: var(--radius); padding: 13px 14px; display: flex; flex-direction: column; gap: 10px; }
  .qhead { display: flex; align-items: baseline; justify-content: space-between; gap: 10px; }
  .qlabel { font-size: 12px; font-weight: 500; color: var(--accent-400); letter-spacing: -0.01em; }
  /* The handle the CLI prints, so `mecha questions show <handle>` reaches the
     same question from a terminal. */
  .qhandle { font-family: var(--mono); font-size: 10px; color: var(--text-muted); }
  .qtask { font-size: 12px; color: var(--text-muted); }
  .qtext { font-size: 14px; line-height: 1.5; }
  .qnote { font-size: 11.5px; color: var(--text-muted); line-height: 1.45; }
  .qwarn { display: flex; gap: 8px; font-size: 11.5px; color: var(--hazard); line-height: 1.45; }
  .qopts { display: flex; gap: 6px; flex-wrap: wrap; }
  .qopt { color: var(--accent-400); border-color: var(--accent-700); }
  /* Loud, and not the hazard colour: nothing is wrong, somebody is waited on. */
  .needs { color: var(--accent-400); border-color: var(--accent-400); }
  /* A run that broke, or one that never said how it ended. Not the agent
     accent and not a chip that reads like the others: the whole rule is that
     this cannot be mistaken for "nothing is happening". */
  .broke { color: var(--hazard); border-color: var(--hazard); }
  /* Finished, with evidence — the quiet end of the set, because nothing is
     wrong and nothing is owed until a person looks. */
  .ready { color: var(--text); border-color: var(--accent-400); }
  .nowline { font-size: 12px; color: var(--accent-400); line-height: 1.45; }
  .evline { font-family: var(--mono); font-size: 10.5px; color: var(--text-muted); line-height: 1.5; }
  .evline.broke { color: var(--hazard); border: none; }
  .warnline { display: flex; gap: 8px; font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .empty { color: var(--text-muted); font-size: 14px; padding: 20px 0; text-align: center; }
  .footnote { font-size: 11px; color: var(--text-muted); text-align: center; padding-top: 6px; }
  .fab { position: absolute; right: 20px; bottom: 20px; width: 56px; height: 56px; border-radius: 14px; background: var(--accent-400); border: none; display: flex; align-items: center; justify-content: center; cursor: pointer; }
  .scrim { position: absolute; inset: 0; z-index: 5; background: rgba(0, 0, 0, 0.45); }
  .namerow { display: flex; gap: 8px; align-items: stretch; }
  .namerow > :global(input) { flex: 1; }
  .menubtn { background: none; border: none; color: var(--text-muted); min-width: 44px; min-height: 44px; margin: -10px 4px -10px -12px; cursor: pointer; display: flex; align-items: center; justify-content: center; }
  .drawer { position: fixed; top: 0; left: 0; bottom: 0; width: min(300px, 82vw); background: var(--bg); border-right: 1px solid var(--accent-700); z-index: 41; display: flex; flex-direction: column; padding-top: env(safe-area-inset-top); animation: drawer-in 0.18s ease-out; }
  @keyframes drawer-in { from { transform: translateX(-100%); } to { transform: translateX(0); } }
  .drawer-head { padding: 18px 16px 12px; border-bottom: 1px solid var(--accent-900); }
  .drawer-title { font-weight: 500; font-size: 16px; letter-spacing: -0.02em; }
  .drawer-scroll { flex: 1; overflow-y: auto; padding: 8px; }
  .drow { display: grid; grid-template-columns: 1fr auto; gap: 2px 8px; width: 100%; text-align: left; background: none; border: none; border-radius: var(--radius); color: var(--text); font: inherit; padding: 11px 10px; min-height: 44px; cursor: pointer; }
  .drow.dactive { background: var(--accent-900); }
  .dname { font-family: var(--mono); font-size: 13px; }
  .dcount { font-size: 13px; font-weight: 500; color: var(--accent-400); text-align: right; }
  .dblurb { grid-column: 1 / -1; font-size: 11px; color: var(--text-muted); }
  .donebtn { color: var(--accent-400); border-color: var(--accent-700); }
  .sheet { position: absolute; left: 0; right: 0; bottom: 0; background: var(--bg); border-top: 1px solid var(--accent-500); border-radius: 16px 16px 0 0; padding: 14px 20px 28px; display: flex; flex-direction: column; gap: 12px; z-index: 6; }
  .grip { width: 36px; height: 4px; border-radius: 2px; background: var(--accent-900); align-self: center; }
  .sheettitle { font-size: 15px; font-weight: 500; }
  .field { background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; padding: 12px 14px; min-height: 44px; box-sizing: border-box; width: 100%; }
  .fieldrow { display: flex; gap: 8px; }
  .btnrow { display: flex; gap: 10px; }
  .btn { flex: 1; min-height: 48px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; }
  .btn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .btn:disabled { opacity: 0.5; }
</style>
