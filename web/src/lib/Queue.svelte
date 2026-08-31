<script module>
  // Grouped listings, kept for as long as the page is open.
  //
  // Module scope on purpose: everything else in this component is per-mount
  // `$state`, and `Review.svelte` renders `<Queue />` inside an `{#if}`, so
  // a tap on the Outbox tab UNMOUNTS this component and takes a listing that
  // cost two minutes of embedding with it. The back arrow out of a grouping
  // did the same by setting `groups = null`. Neither is a cheap thing to
  // throw away: the whole-queue layer embeds every pending statement.
  //
  // Safe to keep, and this is the part that makes it safe rather than
  // convenient — a stale listing already degrades correctly everywhere it is
  // spent. A group verdict sends the ids the card showed and the graph vets
  // them (`vet_cascade_ids`), dropping any decided since without comment;
  // `/api/queue/items` returns only what is still pending and the group
  // screen reports the gap out loud ("3 already judged"). Nothing downstream
  // trusts this listing to be current, so nothing breaks when it is not.
  // What was missing was telling the reviewer, which is why an entry carries
  // the moment it ran and the header offers a regroup.
  //
  // Per page load, not per install: no invalidation rule, no disk, and a
  // refresh is a clean start. A cache with an expiry policy would be this
  // page deciding when the graph has moved, which it cannot know.
  const groupCache = new Map();
  const cacheKey = (spec) =>
    spec.all
      ? `global:${typeof spec.threshold === 'number' ? spec.threshold.toFixed(2) : 'default'}`
      : // Serialised rather than joined on a delimiter: a proposer is free
        // text and a predicate is a graph `cluster_key`, so any character
        // picked as a separator is one they are allowed to contain, and a
        // collision would serve one class's groups under another's name.
        `class:${JSON.stringify([spec.proposer, spec.predicate])}`;
</script>

<script>
  // The graph queue on the phone, at the TUI /queues modal's three depths:
  // proposers → one proposer's classes (with the evidence-tier filter) →
  // either a random sample deck or the class's similarity groups.
  //
  // The sampling rules are the CLI's: the seed is drawn server-side and
  // printed here, a verdict never resamples (the card is dropped locally;
  // these twelve stay one sample), and a new draw is an explicit button. An
  // unjudged class shows a dash, never 0% — "untouched" and "rejected" are
  // opposite findings. Tiers arrive stamped by the server
  // (`tui::queues::Tier::of`, the single definition) and are never
  // re-derived here, where the thresholds would drift.
  //
  // Groups are where one verdict fans out furthest: a group's face is a
  // real member statement, never a paraphrase, and a group verdict is ONE
  // human verdict — the leader is yours, the members follow as a labeled
  // machine cascade the autonomy ladder never counts. A class group never
  // crosses a class; the front screen's "similar across everything" is the
  // invited crossing — stricter floor, every class named on the card.
  // The face of a candidate, and the ONE place this page decides it. The
  // chain is the Rust readers' — `statement`, then `what` for a
  // commitment-shaped payload, then a named absence — the same two keys
  // `tui::queues::items_from_json` looks under, with a test on exactly the
  // commitment case.
  //
  // It used to be spelled inline as
  //     payload.statement ?? `${subject} — ${predicate} — ${object}`
  // which cannot fall through: a template literal is never nullish, so `??`
  // stops at it. A commitment payload carries `{who, what, when, direction}`
  // and no s/p/o at all, so every one of the 695 commitment cards read
  // "undefined — undefined — undefined" and asked for a verdict on a belief
  // nobody could see. That is the outbox's rule arriving in this store: a
  // field the reviewer cannot read is a field they decided unread.
  //
  // Matched to the Rust chain rather than improved on. A third reader of a
  // rule is where it drifts, and a card here that said more than the TUI's
  // for the same candidate would be the drift, not a feature.
  const faceOf = (payload) => payload?.statement ?? payload?.what ?? '(no statement)';

  let proposers = $state(null);
  // The surfaced-verdict queue (review-on-use): live UNREVIEWED facts that
  // are about to matter — served in a pack, contradicting a reviewed fact,
  // or spot-checked by a sampled class. These are facts, not candidates:
  // Confirm stands behind one (tier → reviewed, the discount lifts);
  // Refute retracts it as never true, and the reason feeds the graph's
  // rejection memory. Verdicts land in the owner's own mecha-graph binary
  // via the server — the MCP surface deliberately cannot vote.
  let shadow = $state(null); // { rows, total, live, served }
  // The shadow queue failing to load is a FINDING, not an absence: the CLI
  // row it mirrors answers a dash plus the reason on the same failure, and
  // a card that silently vanishes makes a broken reader look like an
  // install with no shadow queue. Kept separate from `error` so the
  // candidate views survive it.
  let shadowError = $state(null);
  let shadowOpen = $state(false);
  let reasons = $state({}); // fact uid → typed refute reason
  let classes = $state(null); // { proposer, rows }
  let tierFilter = $state(null); // null = all
  let groups = $state(null); // { proposer, predicate, threshold, rows }
  let deck = $state(null); // { proposer, predicate, seed, items, judged }
  let items = $state(null); // one group's members: { from, ids, rows, judged, total }
  let error = $state(null);
  let busy = $state(false);

  const TIERS = ['unjudged', 'thin', 'some', 'solid'];

  async function load() {
    try {
      const res = await fetch('/api/queue');
      if (!res.ok) throw new Error((await res.text()).trim());
      proposers = await res.json();
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
    loadShadow();
  }

  // Beside the queue, failure reported: an older graph binary has no
  // shadow verb, and the candidate views must not vanish with it — but
  // the row must say it could not look, not disappear.
  async function loadShadow() {
    try {
      const res = await fetch('/api/queue/shadow');
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      shadow = {
        rows: data.surfaced ?? [],
        // The graph's pre-truncation count — the page length is not a
        // depth. Older servers without the field get the page as a floor.
        total: data.surfaced_total ?? (data.surfaced ?? []).length,
        live: data.shadow_live ?? 0,
        served: data.shadow_served ?? 0,
      };
      shadowError = null;
    } catch (e) {
      shadow = null;
      shadowError = String(e?.message ?? e);
    }
  }
  load();

  async function shadowVerdict(sf, confirm) {
    busy = true;
    try {
      const res = await fetch('/api/queue/shadow/verdict', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          uid: sf.fact.uid,
          confirm,
          reason: reasons[sf.fact.uid]?.trim() || null,
        }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      shadow.rows = shadow.rows.filter((r) => r !== sf);
      shadow.total = Math.max(0, shadow.total - 1);
      clearNote(sf.fact.uid);
      error = null;
      // The deck refills when this page empties: the fetch was one page of
      // at most ten, and "you judged your page" is not "nothing surfaced".
      // The empty state below renders only when a FRESH fetch returns none.
      if (shadow.rows.length === 0) await loadShadow();
    } catch (e) {
      note(sf.fact.uid, { error: String(e?.message ?? e) });
    } finally {
      busy = false;
    }
  }

  async function openClasses(proposer) {
    try {
      const q = new URLSearchParams({ proposer });
      const res = await fetch(`/api/queue/classes?${q}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      classes = { proposer, rows: await res.json() };
      tierFilter = null;
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  // Write the listing on screen back to the cache under its own key. Called
  // after every local edit to it, because the prunes replace `rows` with a
  // new array — a cache holding the array it was handed at fetch time would
  // hand back members the reviewer has already judged, which is the one way
  // this cache could be worse than no cache.
  //
  // Written to every key that names this listing, not just the one it was
  // opened under. The global layer files an answer twice — under the
  // threshold ASKED for (often "default") and under the one the server says
  // it RAN at — so the stepper can walk back to a floor it has already
  // visited. One entry updated and the other not would mean stepping away
  // and back re-offered a group the reviewer had already emptied, which is
  // the stale-listing failure this cache exists to avoid rather than cause.
  function cacheGroups() {
    if (!groups?.key) return;
    const snap = $state.snapshot(groups);
    for (const [k, v] of groupCache) {
      if (v.key === snap.key) groupCache.set(k, snap);
    }
    groupCache.set(snap.key, snap);
  }

  // Install a cached listing, or fetch one. `force` is the regroup button:
  // the only way to make the queue be embedded again, and an explicit one,
  // because it is minutes.
  async function loadGroups(spec, force = false) {
    const key = cacheKey(spec);
    if (!force) {
      const hit = groupCache.get(key);
      if (hit) {
        // Straight to the listing, with no `rows: null` in between: the
        // placeholder is what makes a cached open still *look* like a wait.
        groups = { ...hit, key };
        error = null;
        return;
      }
    }
    groups = { ...spec, key, threshold: null, rows: null, considered: null, at: null };
    try {
      const q = new URLSearchParams(
        spec.all ? { all: 'true' } : { proposer: spec.proposer, predicate: spec.predicate }
      );
      // Only a real number becomes a param — an event object handed by a
      // bare onclick={openGlobal} must fall through to the server default.
      if (spec.all && typeof spec.threshold === 'number' && isFinite(spec.threshold)) {
        q.set('threshold', spec.threshold.toFixed(2));
      }
      const res = await fetch(`/api/queue/groups?${q}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      // Whoever is on screen now decides whether this answer is still
      // wanted. A grouping runs for minutes, and every other listing is now
      // instant, so a reviewer can step back and open two more classes while
      // the first is still embedding — this used to land on top of whatever
      // they were reading. The answer is still cached below, so the run is
      // not wasted; it just does not seize the screen.
      const wanted = groups?.key === key;
      const next = {
        ...spec,
        key,
        threshold: data.threshold,
        rows: data.groups,
        considered: data.considered ?? null,
        at: Date.now(),
      };
      groupCache.set(key, next);
      // Also under the threshold the server says it RAN at, so stepping back
      // to a floor already visited is a hit. Asking with no threshold and
      // asking for the default are the same answer under two names, and the
      // stepper always steps from the reported value.
      if (spec.all && typeof data.threshold === 'number') {
        groupCache.set(cacheKey({ all: true, threshold: data.threshold }), next);
      }
      if (wanted) {
        groups = { ...next };
        error = null;
      }
    } catch (e) {
      // A failed grouping keeps whatever listing it was asked from rather
      // than clearing it. Both layers embed, and both answer through a
      // stated budget on the server, so a timeout is a thing this page will
      // meet — and trading a good listing for an error message charges the
      // reviewer the whole embedding again to get back to where they were.
      // Reported either way: a regroup that failed must not look like one
      // that found nothing new.
      if (groups?.key !== key) return;
      error = String(e?.message ?? e);
      const hit = groupCache.get(key);
      groups = hit ? { ...hit, key } : null;
    }
  }

  const openGroups = (proposer, predicate, force = false) =>
    loadGroups({ proposer, predicate }, force);

  // The top layer: near-repeats across the WHOLE queue, wherever they sit.
  // Embedding every pending statement takes minutes, and an honest wait
  // message beats a spinner that looks hung — but only the FIRST time at a
  // given floor. Each threshold the stepper visits is remembered, so walking
  // looser and back again is two waits, not four.
  const openGlobal = (threshold = null, force = false) =>
    loadGroups(
      // A bare `onclick={openGlobal}` hands this an event object; only a real
      // number is a threshold, and anything else must fall through to the
      // server default — including in the cache key, or one listing would be
      // filed under a MouseEvent.
      { all: true, threshold: typeof threshold === 'number' && isFinite(threshold) ? threshold : null },
      force
    );

  async function draw(proposer, predicate = null, seed = null) {
    busy = true;
    try {
      const res = await fetch('/api/queue/sample', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ proposer, predicate, seed }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      deck = { proposer, predicate, seed: data.seed, items: data.items, judged: 0, total: data.items.length };
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  // What a card is saying about its own last verdict, keyed by the id that
  // verdict named. Deliberately not one page-level string: the failure this
  // surface kept producing — `cannot resolve subject 'X'` — is answered by
  // two actions on that one candidate, and an error line at the top of the
  // screen is a message a thumb cannot act on. `created` records that the
  // attempt already passed --create-subjects, so the hint offering it does
  // not reappear pointing at what just failed.
  let notes = $state({}); // id → { error?, said?, created? }
  const note = (id, patch) => (notes = { ...notes, [id]: patch });
  const clearNote = (id) => {
    const rest = { ...notes };
    delete rest[id];
    notes = rest;
  };

  // The one place a verdict is sent. Three callers file them — the sample
  // deck, a whole group, one member of a group — and the rule they share is
  // the one that cost a real sitting its arithmetic: **the card leaves only
  // once the server says the verdict landed.** It used to leave on any 2xx,
  // and the graph reports a per-candidate failure on stdout while exiting
  // zero, so a candidate that could not resolve vanished from the sample,
  // stayed pending in the queue, and was counted as one of the twelve
  // verdicts the sitting described. Three copies of that would be three
  // places to lose it again.
  async function sendVerdict(body) {
    const res = await fetch('/api/queue/verdict', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw new Error((await res.text()).trim());
    return res.json();
  }

  async function verdict(accept, create = false) {
    const item = deck.items[0];
    busy = true;
    try {
      await sendVerdict({ id: item.id, accept, create_subjects: create });
      deck.items.shift();
      deck.judged += 1;
      clearNote(item.id);
      error = null;
    } catch (e) {
      note(item.id, { error: String(e?.message ?? e), created: create });
    } finally {
      busy = false;
    }
  }

  // Into a group without covering it.
  //
  // A group verdict is one keystroke over every member, which is right when
  // they repeat and wrong when they merely *rhyme*: a real group of seven
  // near-repeats named Sage, Joseph and Justin — a son and two daughters —
  // and one Accept would have asserted all seven as facts. Similarity is the
  // grouping key, not agreement, so the members have to be tellable apart by
  // hand. The TUI has had this depth since the level existed (`Enter items`);
  // the page had only the two whole-group keys, which is the one shape of
  // this surface where the fast path is the unsafe one.
  //
  // **A named set, re-fetched by id — never a redraw.** The ids are the ones
  // the group card showed, in the order it showed them, leader first: what a
  // person saw is what they are judging. There is deliberately no resample
  // here, for the same reason the deck has none mid-sitting.
  async function openItems(g) {
    const ids = [g.leader_id, ...g.members.map((m) => m[0])];
    items = { from: g, ids, rows: null, judged: 0, total: ids.length };
    try {
      const q = new URLSearchParams({ ids: ids.join(',') });
      const res = await fetch(`/api/queue/items?${q}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      const rows = await res.json();
      // `total` is what CAME BACK, never what was asked for. The store
      // returns pending candidates only, and a member verdicted since the
      // grouping ran — by an earlier cascade, or in another session — is
      // simply not among them. Counting the request would leave the progress
      // bar short of full forever and end on "Every member judged — 9
      // verdicts" under a total of 12, which reads as three verdicts lost:
      // the surface accusing itself of the one failure it exists to prevent.
      //
      // The gap is reported rather than smoothed over, because it is real.
      // (It used to be mostly `--top`'s silent cap of ten, which is fixed in
      // `mecha review items` — a set the caller names is not a listing.)
      items = { from: g, ids, rows, judged: 0, total: rows.length, asked: ids.length };
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
      items = null;
    }
  }

  // One member, one verdict, and deliberately no cascade. Inside a group the
  // members are the thing being told apart, so fanning out from one of them
  // would undo the reason for opening it — and it would spend the owner's
  // one human verdict on candidates they had just decided to read separately.
  async function itemVerdict(item, accept, create = false) {
    busy = true;
    try {
      await sendVerdict({ id: item.id, accept, create_subjects: create });
      items.rows = items.rows.filter((r) => r.id !== item.id);
      items.judged += 1;
      clearNote(item.id);
      error = null;
    } catch (e) {
      note(item.id, { error: String(e?.message ?? e), created: create });
    } finally {
      busy = false;
    }
  }

  // Leaving the group, without paying for the queue a second time.
  //
  // Anything judged in there is gone from the queue, so the listing behind
  // this screen describes members that no longer exist. That used to be
  // answered by re-running the same query, which for the global layer is a
  // two-minute re-embedding of every pending statement — charged on the way
  // OUT of a group the reviewer had just finished reading. A guard skipped it
  // when nothing had been judged, so a *glance* was free and the actual work
  // was not: open a group, reject three, step back, wait two minutes. That is
  // the ordinary loop of a sitting, which made the ordinary loop the
  // expensive one, and a sitting ended when patience did.
  //
  // Nothing here needs re-deriving. The page knows exactly which ids it
  // judged: `itemVerdict` already dropped every verdicted row from
  // `items.rows`, so the survivors ARE the group, in the order it was shown.
  // Removing a member cannot change any other group either — similarity was
  // computed over statements, and a verdict does not move a statement.
  //
  // This is the TUI's rule arriving here rather than a new one: the
  // `Level::Items if from_group` arm of Esc in `tui/mod.rs` has rebuilt the
  // group from its survivors since the level existed, and makes no child
  // call. The web pane was the surface still paying.
  function closeItems() {
    const from = items?.from;
    const ids = items?.ids ?? [];
    const rows = items?.rows;
    items = null;
    // `rows` is null while the fetch is in flight — Back during the load is
    // a real keypress, and treating "not read yet" as "none survived" would
    // delete a group nobody judged. Unknown is not empty.
    if (!from || !Array.isArray(rows) || !groups?.rows?.includes(from)) return;

    const alive = new Set(rows.map((r) => r.id));
    const survivors = ids.filter((id) => alive.has(id));
    // Includes the members that were already judged elsewhere before this
    // group was opened — the `asked > total` gap the items screen reports.
    // Those are gone from the queue too, and the card behind was already
    // describing them.
    if (survivors.length === ids.length) return;

    if (survivors.length === 0) {
      // Every member judged: this is not a group any more, and a card
      // offering "Reject all 0" is worse than no card.
      groups.rows = groups.rows.filter((g) => g !== from);
      cacheGroups();
      return;
    }

    const [leader, ...rest] = survivors;
    const rowOf = (id) => rows.find((r) => r.id === id);
    // Scores are the ones the grouping reported, carried over by id — never
    // re-derived here. A promoted leader keeps the members the embedder put
    // beside the OLD leader, which is exactly the set a cascade would act on.
    const scores = new Map(from.members);
    groups.rows = groups.rows.map((g) =>
      g !== from
        ? g
        : {
            ...from,
            leader_id: leader,
            leader_statement:
              leader === from.leader_id
                ? from.leader_statement
                : faceOf(rowOf(leader)?.payload),
            members: rest.map((id) => [id, scores.get(id) ?? null]),
            // Rebuilt from rows that are still here rather than blanked: a
            // group's face is a real member statement, and these are the
            // real ones. The TUI rebuilds the same three from `modal.items`
            // at the same point — both surfaces, one rule.
            sample: rest.slice(0, 3).map((id) => faceOf(rowOf(id)?.payload)),
            // `classes` is carried over unchanged, deliberately. It counts
            // members per class and a removal cannot be attributed to one:
            // the key is the graph's own `cluster_key`, and re-deriving it
            // in this page would be a third reader of a rule it must not
            // own. Carried over it can only OVERSTATE the span, and the
            // span drives a caution — "expect a whole-group verdict here to
            // overwrite ~1 in 3". A warning is the one field where the
            // stale copy is the safe one.
          }
    );
    cacheGroups();
  }

  // One tap, one human verdict: the leader id is the owner's, the member
  // ids ride as the cascade — always the ids the page showed, never a
  // re-derived similarity.
  //
  // A failed seed cascades nothing, by the graph's own rule (a fan-out from
  // a failed verdict is a fan-out from nothing), so the group stays whole
  // and keeps its place. That is why the ways through belong on the card:
  // binding the leader's subject is usually enough to unblock all of it,
  // because sharing a subject is most of what made it a group.
  async function groupVerdict(g, accept, create = false) {
    busy = true;
    try {
      await sendVerdict({
        id: g.leader_id,
        accept,
        create_subjects: create,
        cascade: g.members.map((m) => m[0]),
        across: !!groups.all,
      });
      groups.rows = groups.rows.filter((r) => r !== g);
      // The listing outlives this screen now, so a verdict has to reach the
      // kept copy too — otherwise stepping out and back in would re-offer
      // "Accept all 7" on a group that is already gone from the queue.
      cacheGroups();
      clearNote(g.leader_id);
      error = null;
    } catch (e) {
      note(g.leader_id, { error: String(e?.message ?? e), created: create });
    } finally {
      busy = false;
    }
  }

  // Rebind an unresolvable subject to a real entity — the graph's own `bind`,
  // which takes its top suggestion and learns the old spelling as an alias,
  // so the fix outlives this one candidate. The row STAYS: a bound subject is
  // a candidate that can now be accepted, not one that has been, and the
  // graph's own line (`#id subject 'old' → New — accept to promote`) is
  // passed through rather than re-worded, next keypress included.
  // What a person typed as an explicit bind target, keyed by candidate.
  let targets = $state({}); // id → display name

  async function bindSubject(id, to = null) {
    busy = true;
    try {
      const res = await fetch('/api/queue/bind', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ id, to: to?.trim() || null }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const data = await res.json();
      note(id, { said: (data.output || '').trim() || 'subject bound — accept to promote' });
    } catch (e) {
      // A bind that failed says nothing about --create-subjects, so whether
      // that hint is still on offer is carried over rather than decided here.
      //
      // `needsTarget` is set by THIS catch and nowhere else, which is what
      // makes the field appear exactly when naming a target is the answer.
      // A failed accept lands in the same note and must not offer it — the
      // ways through an unresolvable subject are bind and create, not
      // typing a name at the tool that never asked for one.
      //
      // Set on any bind failure rather than by matching the graph's wording:
      // every refusal `bind_subject` can give but one ("already resolves")
      // is answered by naming a target, and the child's own sentence is
      // still on screen above the field. Reading the error text to decide
      // would be this page keeping a second copy of the graph's vocabulary,
      // which is how a surface starts disagreeing with the store it shows.
      note(id, { error: String(e?.message ?? e), created: notes[id]?.created, needsTarget: true });
    } finally {
      busy = false;
    }
  }

  function skip() {
    deck.items.push(deck.items.shift());
  }

  const rate = (p) => {
    const judged = p.accepted_hist + p.rejected_hist;
    return judged > 0 ? `${Math.round((p.accepted_hist / judged) * 100)}%` : '—';
  };
  const shownClasses = $derived(
    classes ? classes.rows.filter((c) => !tierFilter || c.tier === tierFilter) : []
  );
</script>

{#snippet hazardGlyph(size = 13)}
  <svg viewBox="0 0 24 24" width={size} height={size} style="flex-shrink: 0" fill="none" stroke="var(--hazard)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 4l9 16H3z" /><path d="M12 11v4M12 17.5v.5" />
  </svg>
{/snippet}

<!-- What one card has to say about its own last verdict, and the two ways
     through the failure that produces almost all of them. Both keys exist in
     the TUI (`b` binds, `A` accepts as a new topic) and neither existed here,
     which is what left a phone holding an error it could not answer. Offered
     after a failure rather than always: `--create-subjects` invents a topic
     node, which is not a default. Suppressed once it has been tried, on the
     TUI's rule — a hint pointing at what just failed is circular. -->
{#snippet verdictNote(id, retryCreating)}
  {#if notes[id]?.error}
    <div class="cardwarn">{@render hazardGlyph()}<span>{notes[id].error}</span></div>
    <div class="ways">
      <button class="ghost" disabled={busy} onclick={() => bindSubject(id)}>Bind subject</button>
      {#if !notes[id].created}
        <button class="ghost" disabled={busy} onclick={retryCreating}>Accept as new topic</button>
      {/if}
    </div>
    <!-- The remedy the graph names — `name a target with --to`. The server
         has always taken it (`BindBody.to`); this page never sent one, so
         the card displayed an instruction it could not carry out. An exact
         display name, because that is what `resolve_entity_all` matches:
         anything ambiguous comes back saying how many it hit. -->
    {#if notes[id].needsTarget}
      <div class="bindrow">
        <input
          class="field"
          placeholder="bind to this entity — exact display name"
          bind:value={targets[id]}
          onkeydown={(e) => e.key === 'Enter' && targets[id]?.trim() && bindSubject(id, targets[id])}
        />
        <button
          class="minibtn"
          disabled={busy || !targets[id]?.trim()}
          onclick={() => bindSubject(id, targets[id])}>Bind</button
        >
      </div>
    {/if}
  {:else if notes[id]?.said}
    <div class="cardsaid">{notes[id].said}</div>
  {/if}
{/snippet}

{#snippet backTo(action, label)}
  <button class="backbtn" onclick={action} aria-label={label}>
    <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6l-6 6 6 6" /></svg>
  </button>
{/snippet}

<div class="pane">
  {#if error}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}

  {#if deck}
    <div class="deckhead">
      {@render backTo(() => { deck = null; }, 'back')}
      <span class="pname">{deck.proposer}{deck.predicate ? ` · ${deck.predicate}` : ''}</span>
      <span class="seed">sample of {deck.total} · seed {deck.seed}</span>
    </div>
    <div class="progress">
      <div class="bar"><div class="fill" style:width="{(deck.judged / deck.total) * 100}%"></div></div>
      <span class="seed">{deck.judged} / {deck.total}</span>
    </div>

    {#if deck.items.length === 0}
      <div class="empty">
        Sample done — {deck.judged} verdicts on one draw.
        <button class="btn" onclick={() => draw(deck.proposer, deck.predicate)}>New draw</button>
      </div>
    {:else}
      {@const item = deck.items[0]}
      <div class="card candidate">
        <div class="kicker">proposed belief</div>
        <div class="statement">{faceOf(item.payload)}</div>
        <div class="meta">
          <span>confidence {item.confidence?.toFixed(2) ?? '—'}</span>
          <span>·</span>
          <span>{item.proposed_by}</span>
          <span>·</span>
          <span>{(item.created_at ?? '').slice(0, 10)}</span>
        </div>
      </div>
      <div class="btnrow">
        <button class="btn" disabled={busy} onclick={() => verdict(false)}>Reject</button>
        <button class="btn primary" disabled={busy} onclick={() => verdict(true)}>Accept</button>
      </div>
      {@render verdictNote(item.id, () => verdict(true, true))}
      <div class="deckfoot">
        <button class="ghost" onclick={skip}>Skip for now</button>
        <button class="ghost" disabled={busy} onclick={() => draw(deck.proposer, deck.predicate)}>New draw</button>
      </div>
      <div class="footnote">These verdicts describe one sample — the seed is printed above.</div>
    {/if}
  {:else if shadowOpen && shadow}
    <div class="deckhead">
      {@render backTo(() => { shadowOpen = false; loadShadow(); }, 'back')}
      <span class="pname">surfaced for verdict</span>
      <span class="seed"
        >{shadow.total} surfaced · {shadow.live} unreviewed live · {shadow.served} ever served</span
      >
    </div>
    {#if shadow.rows.length === 0}
      <div class="empty">Nothing surfaced — no unreviewed fact is about to matter right now.</div>
    {:else}
      <div class="footnote">
        These facts are already live — served rank-discounted and labeled unreviewed. Confirm
        stands behind one; Refute retracts it as never true, and your reason becomes rejection
        memory.
      </div>
      {#each shadow.rows as sf (sf.fact.uid)}
        <div class="card candidate">
          <div class="kicker">{sf.fact.extractor ?? '?'} · {sf.fact.predicate}</div>
          <div class="statement">{sf.fact.statement}</div>
          {#each sf.reasons as r}
            <div class="member">↳ {r}</div>
          {/each}
          <input
            class="field"
            placeholder="refute reason — feeds rejection memory (optional)"
            bind:value={reasons[sf.fact.uid]}
          />
          <div class="btnrow">
            <button class="btn" disabled={busy} onclick={() => shadowVerdict(sf, false)}>Refute</button>
            <button class="btn primary" disabled={busy} onclick={() => shadowVerdict(sf, true)}>Confirm</button>
          </div>
          {#if notes[sf.fact.uid]?.error}
            <div class="cardwarn">{@render hazardGlyph()}<span>{notes[sf.fact.uid].error}</span></div>
          {/if}
        </div>
      {/each}
    {/if}
  {:else if items}
    <div class="deckhead">
      {@render backTo(closeItems, 'back to groups')}
      <span class="pname">one group · {items.total} item{items.total === 1 ? '' : 's'}</span>
      <span class="seed">
        {#if items.asked > items.total}{items.asked - items.total} already judged · {/if}leader
        #{items.from.leader_id}
      </span>
    </div>
    {#if items.judged > 0}
      <div class="progress">
        <div class="bar"><div class="fill" style:width="{(items.judged / items.total) * 100}%"></div></div>
        <span class="seed">{items.judged} / {items.total}</span>
      </div>
    {/if}
    {#if items.rows === null}
      <div class="empty">reading the group…</div>
    {:else if items.rows.length === 0}
      <div class="empty">
        Every member judged — {items.judged} verdicts, one per fact.
        <button class="btn" onclick={closeItems}>Back to groups</button>
      </div>
    {:else}
      <div class="footnote">
        These are near-repeats, not agreements — a group is built on similarity, so members can
        contradict each other. Each verdict here is your own: one fact, no cascade.
      </div>
      {#each items.rows as item (item.id)}
        <div class="card candidate">
          <div class="kicker">#{item.id}{item.id === items.from.leader_id ? ' · leader' : ''}</div>
          <div class="statement">{faceOf(item.payload)}</div>
          <div class="meta">
            <span>confidence {item.confidence?.toFixed(2) ?? '—'}</span>
            <span>·</span>
            <span>{item.proposed_by}</span>
            <span>·</span>
            <span>{(item.created_at ?? '').slice(0, 10)}</span>
          </div>
          <div class="btnrow">
            <button class="btn" disabled={busy} onclick={() => itemVerdict(item, false)}>Reject</button>
            <button class="btn primary" disabled={busy} onclick={() => itemVerdict(item, true)}>Accept</button>
          </div>
          {@render verdictNote(item.id, () => itemVerdict(item, true, true))}
        </div>
      {/each}
    {/if}
  {:else if groups}
    <div class="deckhead">
      {@render backTo(() => { groups = null; }, 'back')}
      <span class="pname">{groups.all ? 'across all classes' : groups.predicate}</span>
      {#if groups.threshold != null}
        <span class="seed">cosine ≥ {groups.threshold.toFixed(2)}</span>
        {#if groups.all}
          <!-- Step from the threshold the envelope says RAN, never from a
               constant of this page's own — the drifted-literal trap. -->
          <button class="stepbtn" title="looser — bigger groups, read more carefully" onclick={() => openGlobal(groups.threshold - 0.03)}>−</button>
          <button class="stepbtn" title="stricter — only near-identical" onclick={() => openGlobal(groups.threshold + 0.03)}>+</button>
        {/if}
      {/if}
    </div>
    <!-- When this listing was embedded, and the one way to embed it again.
         A clock time rather than "4 minutes ago": nothing here ticks, and an
         elapsed figure that only updates when something else re-renders is a
         number that quietly stops being true. The hour it ran is true for as
         long as it is on screen. -->
    {#if groups.rows !== null && groups.at}
      <div class="keptline">
        <span
          >grouped at {new Date(groups.at).toLocaleTimeString([], {
            hour: '2-digit',
            minute: '2-digit',
          })} · kept until you reload the page</span
        >
        <button
          class="ghost regroup"
          disabled={busy}
          onclick={() =>
            groups.all
              ? openGlobal(groups.threshold, true)
              : openGroups(groups.proposer, groups.predicate, true)}
          >Regroup</button
        >
      </div>
    {/if}
    {#if groups.rows === null}
      <div class="empty">
        {groups.all
          ? 'embedding the whole queue — this takes a couple of minutes, stay put'
          : 'grouping by similarity…'}
      </div>
    {:else if groups.rows.length === 0}
      <div class="empty">Nothing repeats above the threshold — review item by item.</div>
    {:else}
      {#if groups.all}
        <div class="footnote">
          {groups.rows.length} groups covering
          {groups.rows.reduce((n, g) => n + g.members.length + 1, 0)} of {groups.considered} pending ·
          singletons stay in their class listings. One tap is one human verdict — the shown
          statement is yours, the rest follow as a labeled machine cascade, and each group names
          every class it touches.
        </div>
        <div class="footnote warn">
          Measured against your own verdict record (2026-08-29): cross-class twins agreed with
          each other only ~63% of the time, at every floor — expect a whole-group verdict here to
          overwrite ~1 in 3. Prefer “Review each” on mixed groups; within-class groups run ~89%.
        </div>
      {:else}
        <div class="footnote">
          A group verdict is one human verdict: the shown statement is yours, the rest follow as a
          labeled machine cascade. A class group never crosses a class — the “everything” layer is
          on the front screen.
        </div>
      {/if}
      {#each groups.rows as g}
        <div class="card candidate">
          <div class="kicker">{g.members.length + 1} near-repeats · leader #{g.leader_id}</div>
          <div class="statement">{g.leader_statement}</div>
          {#if groups.all && g.classes}
            <div class="spans">
              {#each Object.entries(g.classes) as [c, n]}
                <span class="spanchip">{c} ×{n}</span>
              {/each}
            </div>
          {/if}
          {#each g.sample as s}
            <div class="member">≈ {s}</div>
          {/each}
          <div class="btnrow">
            <button class="btn" disabled={busy} onclick={() => groupVerdict(g, false)}>Reject all {g.members.length + 1}</button>
            <button class="btn primary" disabled={busy} onclick={() => groupVerdict(g, true)}>Accept all {g.members.length + 1}</button>
          </div>
          <!-- The third way, and the only safe one when the members disagree:
               read them one at a time. Under the two whole-group buttons
               rather than beside them — it is the slower path, and a row of
               three equal buttons would make the fan-out look like the
               ordinary choice it is not. -->
          <button class="ghost open" disabled={busy} onclick={() => openItems(g)}>
            Review each of the {g.members.length + 1} →
          </button>
          {@render verdictNote(g.leader_id, () => groupVerdict(g, true, true))}
        </div>
      {/each}
    {/if}
  {:else if classes}
    <div class="deckhead">
      {@render backTo(() => { classes = null; load(); }, 'back to proposers')}
      <span class="pname">{classes.proposer}</span>
      <span class="seed">{shownClasses.length} of {classes.rows.length} classes</span>
    </div>
    <div class="tierchips">
      <button class="tchip" class:on={tierFilter === null} onclick={() => (tierFilter = null)}>all</button>
      {#each TIERS as t}
        <button class="tchip" class:on={tierFilter === t} onclick={() => (tierFilter = t)}>{t}</button>
      {/each}
    </div>
    {#each shownClasses as c}
      <div class="card row">
        <div class="rowtop">
          <span class="pname">{c.predicate}</span>
          <span class="chip">{c.tier}</span>
          <span class="pcount">{c.pending.toLocaleString('en-US')}</span>
        </div>
        {#if c.samples?.length}<div class="sample">{c.samples[0]}</div>{/if}
        <div class="rowsub">
          <span>your accept rate {rate(c)} over {c.accepted_hist + c.rejected_hist} verdicts</span>
        </div>
        <div class="rowbtns">
          <button class="minibtn" disabled={busy} onclick={() => draw(classes.proposer, c.predicate)}>Sample 12</button>
          <button class="minibtn" disabled={busy} onclick={() => openGroups(classes.proposer, c.predicate)}>Similar groups</button>
        </div>
      </div>
    {:else}
      <div class="empty">No classes in this tier.</div>
    {/each}
  {:else if proposers === null && !error}
    <div class="empty">reading the queue…</div>
  {:else if proposers}
    {#if shadow}
      <button class="card row global" onclick={() => (shadowOpen = true)} disabled={busy}>
        <div class="rowtop">
          <span class="pname">surfaced for verdict</span>
          <span class="pcount">{shadow.total.toLocaleString('en-US')}</span>
        </div>
        <div class="rowsub">
          <span
            >unreviewed facts about to matter — served, contradicting, or spot-checked ·
            {shadow.live.toLocaleString('en-US')} live in the shadow tier</span
          >
        </div>
      </button>
    {:else if shadowError}
      <div class="card row global">
        <div class="rowtop">
          <span class="pname">surfaced for verdict</span>
          <span class="pcount">—</span>
        </div>
        <div class="rowsub"><span>could not read the shadow queue: {shadowError}</span></div>
      </div>
    {/if}
    <button class="card row global" onclick={() => openGlobal()} disabled={busy}>
      <div class="rowtop">
        <span class="pname">similar across everything</span>
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="var(--accent-400)" stroke-width="1.8" stroke-linecap="round"><path d="M13 3L4 14h6l-1 7 9-11h-6z" /></svg>
      </div>
      <div class="rowsub">
        <span>near-repeats grouped over the whole queue, wherever they sit — the fast way through {proposers.reduce((n, p) => n + p.pending, 0).toLocaleString('en-US')} pending</span>
      </div>
    </button>
    {#each proposers as p}
      <button class="card row" onclick={() => openClasses(p.proposer)} disabled={busy}>
        <div class="rowtop">
          <span class="pname">{p.proposer}</span>
          <span class="chip">{p.tier}</span>
          <span class="pcount">{p.pending.toLocaleString('en-US')}</span>
        </div>
        <div class="rowsub">
          <span>your accept rate {rate(p)} over {p.accepted_hist + p.rejected_hist} verdicts</span>
          <span class="dim">{p.classes} class{p.classes === 1 ? '' : 'es'}</span>
        </div>
      </button>
    {:else}
      <div class="empty">The queue is empty.</div>
    {/each}
  {/if}
</div>

<style>
  .pane { display: flex; flex-direction: column; gap: 10px; }
  .cardwarn { display: flex; gap: 8px; align-items: flex-start; font-size: 12px; color: var(--hazard); line-height: 1.45; padding-top: 2px; }
  .cardsaid { font-family: var(--mono); font-size: 11px; color: var(--text-muted); line-height: 1.5; padding-top: 2px; }
  .ways { display: flex; gap: 8px; flex-wrap: wrap; }
  .ways .ghost { flex: 1; min-height: 42px; border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); }
  .warnline { display: flex; align-items: flex-start; gap: 8px; font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .row { text-align: left; padding: 14px; display: flex; flex-direction: column; gap: 7px; cursor: pointer; color: var(--text); font: inherit; }
  .rowtop { display: flex; align-items: center; gap: 8px; }
  .pname { font-family: var(--mono); font-size: 13px; color: var(--accent-400); overflow-wrap: anywhere; }
  .pcount { font-size: 16px; font-weight: 500; margin-left: auto; }
  .rowsub { display: flex; justify-content: space-between; font-size: 11px; color: var(--text-muted); }
  .dim { color: var(--accent-700); }
  .sample { font-size: 12px; color: var(--text-muted); line-height: 1.45; overflow: hidden; display: -webkit-box; -webkit-box-orient: vertical; -webkit-line-clamp: 2; line-clamp: 2; }
  .rowbtns { display: flex; gap: 8px; }
  .minibtn { flex: 1; min-height: 40px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); color: var(--text); font-size: 12px; cursor: pointer; }
  .minibtn:disabled { opacity: 0.5; }
  .tierchips { display: flex; gap: 6px; flex-wrap: wrap; }
  .tchip { font-family: var(--mono); font-size: 11px; color: var(--text-muted); background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); padding: 7px 11px; min-height: 34px; cursor: pointer; }
  .tchip.on { color: var(--text); background: var(--accent-900); border-color: var(--accent-700); }
  .empty { color: var(--text-muted); font-size: 14px; padding: 20px 0; text-align: center; display: flex; flex-direction: column; gap: 12px; align-items: center; }
  .deckhead { display: flex; align-items: center; gap: 10px; }
  .backbtn { background: none; border: none; color: var(--text-muted); min-width: 44px; min-height: 44px; margin: -10px 0 -10px -12px; cursor: pointer; display: flex; align-items: center; justify-content: center; }
  .seed { font-family: var(--mono); font-size: 11px; color: var(--text-muted); margin-left: auto; }
  .progress { display: flex; align-items: center; gap: 10px; }
  .bar { flex: 1; height: 3px; background: var(--accent-900); border-radius: 2px; overflow: hidden; }
  .fill { height: 3px; background: var(--accent-400); }
  .candidate { padding: 18px; display: flex; flex-direction: column; gap: 12px; background: var(--surface); border-color: var(--accent-700); }
  .statement { font-size: 15px; line-height: 1.5; }
  .member { font-size: 12px; line-height: 1.45; color: var(--text-muted); }
  .global { border-color: var(--accent-700); }
  .spans { display: flex; gap: 6px; flex-wrap: wrap; }
  .spanchip { font-family: var(--mono); font-size: 10px; color: var(--accent-400); background: var(--accent-900); border-radius: var(--radius-chip); padding: 4px 8px; }
  .stepbtn { font-family: var(--mono); font-size: 15px; color: var(--text); background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); min-width: 34px; min-height: 34px; cursor: pointer; }
  .meta { display: flex; gap: 8px; font-family: var(--mono); font-size: 10px; color: var(--text-muted); }
  .btnrow { display: flex; gap: 10px; }
  .btn { flex: 1; min-height: 52px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 15px; cursor: pointer; }
  .btn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .btn:disabled { opacity: 0.5; }
  .deckfoot { display: flex; justify-content: space-between; }
  .ghost { background: none; border: none; color: var(--text-muted); font-size: 13px; min-height: 44px; cursor: pointer; }
  .keptline { display: flex; align-items: center; gap: 10px; font-size: 11px; color: var(--text-muted); }
  .regroup { margin-left: auto; font-size: 11px; min-height: 34px; color: var(--accent-400); }
  .footnote { font-size: 11px; color: var(--text-muted); text-align: center; }
  .footnote.warn { color: var(--hazard); }
  .open { align-self: center; font-size: 13px; color: var(--accent-400); }
  .bindrow { display: flex; gap: 8px; }
  .field { flex: 1; min-height: 42px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 13px; padding: 0 12px; }
  .bindrow .minibtn { flex: 0 0 84px; }
</style>
