<script>
  // #mail/rules (MAIL-RULES-DESIGN.md §8): the owner's view of the mail rule
  // store, inside Mail because it is watched daily. Every change is a
  // `mecha mail rule …` child of mecha serve (P3-1's routes), so this page
  // can do nothing the CLI cannot; the store's gates decide, and the page
  // shows what they said.
  //
  // Three screens — the list, one rule, a new rule — and two sheets: the
  // approval screen a raise to auto goes through (§6.2), and a reader for
  // the thread a ledger row points at.
  import { onMount } from 'svelte';
  import { apiFetch as fetch } from './api.js';
  import { parseThread } from './mail-thread.js';
  import MailBody from './MailBody.svelte';
  // The page's buttons are the settings form's (.tf-*): import them here
  // rather than lean on another component having pulled them into the bundle.
  import './form.css';
  import { takeRuleOffer } from './mail-rules-handoff.svelte.js';
  import {
    GROUPS,
    MODES,
    modeOf,
    isRaise,
    groupLabel,
    outcomeLabel,
    outcomeTone,
    pct,
    verbOf,
    verdictLabel,
    measuredBuckets,
    digestSections,
    messageLine,
    matchedBecause,
    prefillFromThread,
    draftToml,
    draftProblem,
    nameProblem,
    errorOf,
    when,
    DRAFT_VERBS,
  } from './mail-rules.js';

  let { navigate = () => {} } = $props();

  // The ledger list is a window; the counts the approval screen shows are
  // not — they are the store's statistics over every row (review of #850).
  const LEDGER_LIMIT = 200;

  let screen = $state('list'); // list | rule | new
  let rules = $state(null);
  let listError = $state(null);
  let name = $state(null); // the open rule
  let detail = $state(null);
  let ledger = $state(null);
  let detailError = $state(null);
  let busy = $state(false);
  let notice = $state(null); // { text, tone }
  let confirmPauseAll = $state(false);

  // definition editor
  let editText = $state('');
  let editError = $state(null);

  // approval sheet (§6.2)
  let approval = $state(null); // { group, mode, inputs, error }

  // test against recent mail
  let testing = $state(null); // null | { status: 'running' } | { status: 'done', result } | { status: 'error', text }

  // ledger row being read, and its thread
  let peek = $state(null); // { row, status, text }
  // Which request may still write `peek` / `testing`. Plain counters, not
  // $state: an object put in $state comes back as a proxy, so identity
  // checks against it never match.
  let peekSeq = 0;
  let testSeq = 0;

  // new rule
  let draft = $state(prefillFromThread({}));
  let draftName = $state('new-rule');
  let draftText = $state('');
  let draftEdited = $state(false);
  let draftError = $state(null);

  async function call(path, body) {
    const res = await fetch(path, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body ?? {}),
    });
    const text = await res.text();
    if (!res.ok) throw errorOf(res.status, text);
    try {
      return JSON.parse(text);
    } catch {
      return {};
    }
  }

  async function get(path) {
    const res = await fetch(path);
    const text = await res.text();
    if (!res.ok) throw errorOf(res.status, text);
    return JSON.parse(text);
  }

  const say = (text, tone = 'ok') => (notice = { text, tone });
  const enc = encodeURIComponent;

  async function loadList() {
    listError = null;
    try {
      rules = await get('/api/mail/rules');
    } catch (e) {
      listError = e.message;
    }
  }

  // Like the test and the thread sheet, a rule's load answers only the rule
  // still open: a late answer for the rule just left would put its
  // definition under this one's name, and Save would write it there.
  // A reload of the same rule keeps an unsaved edit, unless it follows a save.
  let ruleSeq = 0;
  async function loadRule(n = name, { afterSave = false } = {}) {
    const seq = ++ruleSeq;
    const mine = () => ruleSeq === seq && name === n;
    detailError = null;
    try {
      const [d, l] = await Promise.all([
        get(`/api/mail/rules/${enc(n)}`),
        get(`/api/mail/rules/ledger?rule=${enc(n)}&limit=${LEDGER_LIMIT}`),
      ]);
      if (!mine()) return;
      const unsaved = !afterSave && detail?.name === n && editText !== (detail.rule_toml ?? '');
      detail = d;
      ledger = l;
      if (!unsaved) {
        editText = d.rule_toml ?? '';
        editError = null;
      }
    } catch (e) {
      if (mine()) detailError = e.message;
    }
  }

  function openRule(n) {
    name = n;
    detail = null;
    ledger = null;
    testing = null;
    notice = null;
    screen = 'rule';
    loadRule(n);
  }

  function back() {
    screen = 'list';
    notice = null;
    loadList();
  }

  function openNew(thread = null) {
    draft = prefillFromThread(thread ?? {});
    draftName = draft.name;
    draftEdited = false;
    draftError = null;
    draftText = draftToml(draft);
    screen = 'new';
  }

  onMount(() => {
    const offer = takeRuleOffer();
    if (offer) openNew(offer);
    loadList();
  });

  // The guided fields write the TOML until the owner types in it; after that
  // the text is theirs and the fields stop overwriting it.
  $effect(() => {
    const text = draftToml(draft);
    if (!draftEdited) draftText = text;
  });

  // ---- mutations ----

  async function setMode(group, target) {
    const g = detail?.groups?.find((x) => x.group === group);
    if (!g) return;
    // The store measures a raise from the *effective* mode: re-approving a
    // braked or lapsed group (recorded auto, running shadow) is a raise.
    if (isRaise(g.effective_mode, target)) {
      approval = { group, mode: target, inputs: detail.inputs ?? null, error: null };
      return;
    }
    busy = true;
    try {
      const r = await call(`/api/mail/rules/${enc(name)}/mode`, { group, mode: target });
      say(r.clamped ? `Clamped to ${r.effective_mode ?? 'shadow'}: ${r.actor_reason ?? 'not attributed to you'}` : `${groupLabel(group)} is now ${r.recorded_mode ?? target}.`);
    } catch (e) {
      say(e.message, 'bad');
    }
    busy = false;
    await loadRule();
  }

  async function approve() {
    if (!approval?.inputs?.digest) return;
    busy = true;
    try {
      const r = await call(`/api/mail/rules/${enc(name)}/mode`, {
        group: approval.group,
        mode: approval.mode,
        digest: approval.inputs.digest,
      });
      approval = null;
      say(`${groupLabel(r.group ?? '')} is now ${r.recorded_mode ?? '…'}${r.effective_mode && r.effective_mode !== r.recorded_mode ? `, running ${r.effective_mode}` : ''}.`);
      await loadRule();
    } catch (e) {
      if (e.code === 'stale_digest') {
        // Never retry a stale approval: show the files as they are now.
        await loadRule();
        approval = { ...approval, inputs: detail?.inputs ?? null, error: 'The rule changed since it was shown. Review it again below.' };
      } else {
        approval = { ...approval, error: e.message };
      }
    }
    busy = false;
  }

  async function pauseRule(n) {
    busy = true;
    try {
      const r = await call(`/api/mail/rules/${enc(n)}/pause`);
      say(r.groups ? `Paused ${n}: ${r.groups} group${r.groups === 1 ? '' : 's'} to shadow.` : `${n} was already at shadow or below.`);
    } catch (e) {
      say(e.message, 'bad');
    }
    busy = false;
    if (screen === 'rule') await loadRule();
    else await loadList();
  }

  async function pauseAll() {
    confirmPauseAll = false;
    busy = true;
    try {
      const r = await call('/api/mail/rules/pause-all');
      say(`Paused every rule: ${r.groups ?? 0} group${r.groups === 1 ? '' : 's'} to shadow.`);
    } catch (e) {
      say(e.message, 'bad');
    }
    busy = false;
    await loadList();
  }

  async function saveEdit() {
    // The editor holds the open rule's file, and only that rule is written.
    if (detail?.name !== name) return;
    busy = true;
    editError = null;
    try {
      const r = await call(`/api/mail/rules/${enc(name)}/edit`, { rule_toml: editText });
      if (!r.changed_files?.length) say('No change to save.');
      else if (r.lapsed_groups?.length)
        say(`Saved. Approval lapsed for ${r.lapsed_groups.map(groupLabel).join(', ')}: the files changed, so raise again to approve them.`, 'held');
      else say('Saved.');
      await loadRule(name, { afterSave: true });
    } catch (e) {
      editError = e.message;
    }
    busy = false;
  }

  async function install() {
    // Hand-edited text is the store's to judge; the fields are checked only
    // while they still write it.
    draftError = nameProblem(draftName) ?? (draftEdited ? null : draftProblem(draftName, draft));
    if (draftError) return;
    busy = true;
    try {
      await call('/api/mail/rules/add', { name: draftName, rule_toml: draftText });
      busy = false;
      // openRule clears the notice (a navigation), so say it after.
      openRule(draftName);
      say(`Installed ${draftName} in shadow. It records what it would have done; nothing acts until you raise it.`);
      return;
    } catch (e) {
      draftError = e.message;
    }
    busy = false;
  }

  async function undo(row) {
    busy = true;
    try {
      await call('/api/mail/rules/undo', { row_id: row.id });
      say('Restored.');
    } catch (e) {
      say(e.message, 'bad');
    }
    busy = false;
    await loadRule();
  }

  async function wrongMatch(row) {
    busy = true;
    try {
      const r = await call('/api/mail/rules/wrong-match', { row_id: row.id });
      say(r.message ?? 'Recorded.', 'held');
    } catch (e) {
      say(e.message, 'bad');
    }
    busy = false;
    await loadRule();
  }

  // A test runs for minutes and the owner may open another rule meanwhile:
  // its answer lands only on the rule it tested, never under another name,
  // because that count is what a raise is decided on.
  async function runTest() {
    const forRule = name;
    const seq = ++testSeq;
    testing = { status: 'running' };
    const mine = () => testSeq === seq && name === forRule && screen === 'rule';
    try {
      const result = await call(`/api/mail/rules/${enc(forRule)}/test`, {});
      if (mine()) testing = { status: 'done', result };
    } catch (e) {
      if (!mine()) return;
      // A held or failed account answers 409 with its report: keep it.
      testing = e.body?.accounts ? { status: 'done', result: e.body } : { status: 'error', text: e.message };
    }
  }

  // A read answers only the sheet that asked for it: dismissed, or replaced by
  // another row's, the late answer is dropped rather than reopening a sheet.
  async function openPeek(row) {
    const seq = ++peekSeq;
    peek = { row, status: 'loading', text: '' };
    // A dismissed sheet is null, and stays closed when the read lands.
    const mine = () => peekSeq === seq && peek !== null;
    try {
      const q = new URLSearchParams({ thread: row.thread_id, account: row.account });
      const res = await fetch(`/api/mail/read?${q}`);
      const text = await res.text();
      if (mine()) peek = { row, status: res.ok ? 'ok' : 'error', text: res.ok ? text : text.trim() || `HTTP ${res.status}` };
    } catch (e) {
      if (mine()) peek = { row, status: 'error', text: String(e) };
    }
  }

  function editDefinition() {
    document.getElementById('rule-definition')?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }

  // ---- what is on screen ----

  const groups = $derived((detail?.groups ?? []).filter((g) => g));
  const sections = $derived(digestSections(ledger ?? [], groups));
  const anyActive = (r) => (r.groups ?? []).some((g) => ['propose', 'auto'].includes(modeOf(g.effective_mode)) || ['propose', 'auto'].includes(modeOf(g.recorded_mode)));
  // The store's own counts under the current match digest, per evidence
  // kind (§6.2) — never a count of the rows the ledger window happens to hold.
  const approvalBuckets = $derived(approval ? measuredBuckets(groups.find((g) => g.group === approval.group)) : []);
  const parsedPeek = $derived(peek?.status === 'ok' ? parseThread(peek.text) : null);
  // The read's own subject once it is in; the ledger's (or its pointer) until then.
  const peekTitle = $derived(parsedPeek?.header.find(([k]) => k === 'subject')?.[1] || (peek ? messageLine(peek.row).what : ''));
</script>

<svelte:window onkeydown={(e) => {
  if (e.key !== 'Escape') return;
  if (peek) peek = null;
  else if (approval && !busy) approval = null;
}} />

{#snippet modeChips(g)}
  {@const rec = modeOf(g.recorded_mode)}
  {@const eff = modeOf(g.effective_mode)}
  <span class="chip mode {eff}" title={rec !== eff ? `recorded ${rec}, running ${eff}` : eff}>
    {groupLabel(g.group)} · {eff}{#if rec !== eff}<span class="was"> (set {rec})</span>{/if}
  </span>
{/snippet}

{#snippet holdLines(holds)}
  {#each holds ?? [] as h}<div class="hold">held: {h}</div>{/each}
{/snippet}

{#snippet noticeLine()}
  {#if notice}
    <div class="notice {notice.tone}" role="status">
      <span class="grow">{notice.text}</span>
      <button class="x" aria-label="Dismiss" onclick={() => (notice = null)}>✕</button>
    </div>
  {/if}
{/snippet}

<div class="rulespage">
  {#if screen === 'list'}
    <header class="head">
      <button class="backlink" onclick={() => navigate('mail')}>‹ Mail</button>
      <h1>Rules</h1>
      <span class="grow"></span>
      <button class="tf-btn" disabled={busy} onclick={() => openNew()}>New rule</button>
    </header>
    {@render noticeLine()}
    {#if listError}
      <div class="warnline">Could not read the rules: {listError}</div>
    {:else if rules === null}
      <div class="empty">reading the rules…</div>
    {:else if rules.length === 0}
      <div class="intro">
        <p><strong>No rules yet.</strong> A rule matches mail by its headers — sender, domain, list, subject — and does something with it: archive, mark read, tag, add to the digest, or trash.</p>
        <p>Every new rule starts in <em>shadow</em>: it records what it would have done, and nothing happens to your mail until you raise it. Open a thread and choose <em>Make a rule</em>, or start from nothing.</p>
        <button class="tf-btn primary" onclick={() => openNew()}>New rule</button>
      </div>
    {:else}
      <div class="cards">
        {#each rules as r (r.name)}
          <div class="card">
            <button class="cardmain" onclick={() => openRule(r.name)}>
              <div class="cardtop">
                <span class="rname">{r.name}</span>
                {#if r.description}<span class="desc">{r.description}</span>{/if}
              </div>
              {#if r.load_error}<div class="hold">definition unreadable: {r.load_error}</div>{/if}
              <div class="chips">
                {#each r.groups ?? [] as g}{@render modeChips(g)}{/each}
              </div>
              {@render holdLines(r.holds)}
              {#each r.groups ?? [] as g}
                {#if g.holds?.length}<div class="hold">{groupLabel(g.group)} held: {g.holds.join('; ')}</div>{/if}
              {/each}
              <div class="facts">
                <span>{r.fires_7d ?? 0} fired this week</span>
                <span>last {when(r.last_fired)}</span>
                {#if r.failures}<span class="bad">{r.failures} failed</span>{/if}
                {#each (r.groups ?? []).filter((g) => g.group === 'mailbox') as g}
                  {#each measuredBuckets(g) as b}<span>undone {pct(b.undo_rate)} · {verdictLabel(b.verdict_input)}</span>{/each}
                {/each}
              </div>
            </button>
            {#if anyActive(r)}
              <button class="pausebtn" disabled={busy} onclick={() => pauseRule(r.name)} title="Lower every group to shadow. Resuming is a raise, with its approval screen.">Pause</button>
            {/if}
          </div>
        {/each}
      </div>
      <div class="footer">
        {#if confirmPauseAll}
          <span class="grow">Lower every rule's groups to shadow? Groups already off stay off.</span>
          <button class="tf-btn" onclick={() => (confirmPauseAll = false)}>Cancel</button>
          <button class="tf-btn primary" disabled={busy} onclick={pauseAll}>Pause all</button>
        {:else}
          <span class="grow"></span>
          <button class="tf-btn" disabled={busy} onclick={() => (confirmPauseAll = true)}>Pause all…</button>
        {/if}
      </div>
    {/if}
  {:else if screen === 'new'}
    <header class="head">
      <button class="backlink" onclick={back}>‹ Rules</button>
      <h1>New rule</h1>
    </header>
    <div class="tf">
      <section class="tf-sec">
        <header><h3>What it matches</h3><p class="tf-help">One key here; edit the text below to combine keys (they AND together; a comma list ORs).</p></header>
        <div class="tf-row stack">
          <div class="tf-seg" role="radiogroup" aria-label="Match on">
            {#each [['from_domain', 'Domain'], ['from', 'Sender'], ['list_id', 'List id'], ['subject_contains', 'Subject']] as [k, label]}
              <button class:on={draft.match === k} role="radio" aria-checked={draft.match === k} onclick={() => (draft.match = k)}>{label}</button>
            {/each}
          </div>
          <input class="tf-text mono" aria-label="Match value" placeholder={draft.match === 'list_id' ? 'news.example.com' : draft.match === 'from' ? 'sender@example.com' : draft.match === 'subject_contains' ? 'weekly digest' : 'example.com'} bind:value={draft[draft.match]} />
          {#if draft.match === 'list_id'}<p class="tf-help">The list id is in the message's List-Id header; the mail views do not show it, so type it here.</p>{/if}
        </div>
      </section>
      <section class="tf-sec">
        <header><h3>What it does</h3></header>
        <div class="tf-row stack">
          <div class="tf-seg wrap" role="radiogroup" aria-label="Action">
            {#each DRAFT_VERBS as v}
              <button class:on={draft.verb === v} role="radio" aria-checked={draft.verb === v} onclick={() => (draft.verb = v)}>{v}</button>
            {/each}
          </div>
          {#if draft.verb === 'tag'}<input class="tf-text mono" aria-label="Tag" placeholder="lists" bind:value={draft.tag} />{/if}
          {#if draft.verb === 'trash'}<p class="tf-help">Trash is restorable from the ledger, but archive is the gentler start.</p>{/if}
        </div>
      </section>
      <section class="tf-sec">
        <header><h3>Name and description</h3></header>
        <div class="tf-row stack">
          <input class="tf-text mono" aria-label="Rule name" bind:value={draftName} />
          <input class="tf-text" aria-label="Description" bind:value={draft.description} />
        </div>
      </section>
      <section class="tf-sec">
        <header><h3>rule.toml</h3><p class="tf-help">What will be installed. Typing here takes over from the fields above.</p></header>
        <div class="tf-row stack">
          <textarea class="toml" spellcheck="false" aria-label="rule.toml" bind:value={draftText} oninput={() => (draftEdited = true)}></textarea>
          {#if draftEdited}<button class="linkish" onclick={() => { draftEdited = false; draftText = draftToml(draft); }}>Rewrite from the fields</button>{/if}
        </div>
      </section>
      {#if draftError}<div class="warnline">{draftError}</div>{/if}
      <div class="tf-bar">
        <span class="tf-count">Installs in shadow: nothing acts until you raise it.</span>
        <button class="tf-btn" onclick={back}>Cancel</button>
        <button class="tf-btn primary" disabled={busy} onclick={install}>Install</button>
      </div>
    </div>
  {:else}
    <header class="head">
      <button class="backlink" onclick={back}>‹ Rules</button>
      <h1 class="mono">{name}</h1>
      <span class="grow"></span>
      {#if detail && anyActive(detail)}<button class="tf-btn" disabled={busy} onclick={() => pauseRule(name)}>Pause</button>{/if}
    </header>
    {@render noticeLine()}
    {#if detailError}
      <div class="warnline">Could not read {name}: {detailError}</div>
    {:else if !detail}
      <div class="empty">reading the rule…</div>
    {:else}
      {#if detail.description}<p class="lede">{detail.description}</p>{/if}
      {#if detail.load_error}<div class="warnline">The definition does not load: {detail.load_error}. Fix it below; until then every group stays at shadow or lower.</div>{/if}
      {@render holdLines(detail.holds)}

      <section class="block">
        <h2>Modes</h2>
        <p class="help">Per action group. Lowering is immediate. Raising shows what you approve; an edit to the rule later lapses the approval.</p>
        {#each groups as g (g.group)}
          {@const rec = modeOf(g.recorded_mode)}
          {@const eff = modeOf(g.effective_mode)}
          <div class="grouprow">
            <div class="glabel">
              <span class="gname">{groupLabel(g.group)}</span>
              <span class="gstate">{#if rec !== eff}set {rec}, running <strong>{eff}</strong>{:else}<strong>{eff}</strong>{/if}{#if g.activated_at} · since {when(g.activated_at)}{/if}</span>
              {#each g.holds ?? [] as h}<span class="hold">held: {h}</span>{/each}
            </div>
            <div class="tf-seg" role="radiogroup" aria-label="{groupLabel(g.group)} mode">
              {#each MODES as m}
                <button class:on={rec === m} role="radio" aria-checked={rec === m} disabled={busy || !GROUPS.includes(g.group)} onclick={() => !(rec === m && eff === m) && setMode(g.group, m)}>{m}</button>
              {/each}
            </div>
            {#each measuredBuckets(g) as b}
              <div class="measure">
                <span class="mk">{verdictLabel(b.verdict_input)}{#if b.shadow_upper_bound} · shadow rates are an upper bound{/if}</span>
                <span>{b.matched_actions} matched · {b.matching_threads} threads</span>
                {#if g.group === 'mailbox'}<span>undone {pct(b.undo_rate)} of {b.automatic_mailbox_actions} automatic</span>{/if}
                <span>wrong match {pct(b.wrong_match_rate)}</span>
              </div>
            {/each}
          </div>
        {/each}
      </section>

      <section class="block">
        <h2>Test against recent mail</h2>
        <p class="help">Runs this rule over the last 30 days, every folder, archived mail included. Reads only; nothing is written.</p>
        {#if testing?.status === 'running'}
          <div class="empty">searching 30 days of mail — this can take a few minutes…</div>
        {:else}
          <button class="tf-btn" onclick={runTest}>{testing ? 'Test again' : 'Test'}</button>
        {/if}
        {#if testing?.status === 'error'}<div class="warnline">{testing.text}</div>{/if}
        {#if testing?.status === 'done'}
          {#each testing.result.accounts ?? [] as a}
            <div class="testacct">
              <div class="testhead">
                <strong>{a.account}</strong>
                {#if a.status === 'complete'}<span>{a.matched_rows} would have matched</span>
                {:else if a.status === 'held'}<span class="heldword">held — no count</span>
                {:else if a.status === 'failed'}<span class="bad">incomplete — {a.read_failures + a.candidate_failures} read{a.read_failures + a.candidate_failures === 1 ? '' : 's'} failed</span>
                {:else}<span class="bad">unknown — no count</span>{/if}
              </div>
              {#if a.unknown_header_messages}
                <!-- Unknown is not a non-match (the 2026-11-06 ruling): say how
                     many messages could not be checked, beside the count. -->
                <div class="hold">{a.unknown_header_messages} message{a.unknown_header_messages === 1 ? '' : 's'} could not be checked: the header this rule matches on was not visible ({a.account}'s provider returns headers incompletely).</div>
              {/if}
              {#each a.holds ?? [] as h}<div class="hold">{h}</div>{/each}
              {#each (a.rows ?? []).slice(0, 50) as row (row.id)}
                {@const m = messageLine(row)}
                <div class="lrow compact">
                  <span class="when">{when(row.received_at ?? row.at)}</span>
                  <span class="msg"><span class="who">{m.who}</span> <span class="what">{m.what}</span></span>
                  <span class="chip {outcomeTone(row.outcome)}">{outcomeLabel(row)}{#if row.headers_partial} · partial headers{/if}</span>
                </div>
              {/each}
              {#if (a.rows ?? []).length > 50}<div class="help">and {a.rows.length - 50} more</div>{/if}
            </div>
          {/each}
        {/if}
      </section>

      <section class="block">
        <h2>Ledger</h2>
        {#if ledger?.length >= LEDGER_LIMIT}
          <p class="help">The newest {LEDGER_LIMIT} rows. The counts above cover every row; <code>mecha mail rule ledger {name} --limit N</code> reads further back.</p>
        {/if}
        {#if !ledger?.length}
          <p class="help">Nothing yet. The rules timer runs every five minutes once the rules feature and its timer are on.</p>
        {:else}
          {#each [{ heading: null, rows: sections.current }, ...sections.earlier.map((s) => ({ heading: `${groupLabel(s.group)} · before the matcher changed (newest ${when(s.newest)})`, rows: s.rows }))] as sec}
            {#if sec.heading}<h3 class="sechead">{sec.heading}</h3>{/if}
            {#each sec.rows as row (row.id)}
              {@const m = messageLine(row)}
              <div class="lrow">
                <div class="lmain">
                  <span class="when">{when(row.at)}</span>
                  {#if row.outcome === 'approval_lapsed'}
                    <!-- A rule-level row (§8): it names no message. -->
                    <span class="msg"><span class="who">The rule's files changed</span> <span class="what">— automatic groups dropped until you approve them again</span></span>
                  {:else if row.thread_id && row.account}
                    <button class="msg linkish" onclick={() => openPeek(row)}><span class="who">{m.who}</span> <span class="what">{m.what}</span></button>
                  {:else}
                    <span class="msg"><span class="who">{m.who}</span> <span class="what">{m.what}</span></span>
                  {/if}
                </div>
                <div class="ldetail">
                  <span class="chip {outcomeTone(row.outcome)}">{outcomeLabel(row)}</span>
                  {#if row.outcome !== 'approval_lapsed'}<span>{verbOf(row)} · {groupLabel(row.group)} · {modeOf(row.mode)}</span>{/if}
                  {#if row.headers_partial}<span class="note">matched on a partial header set</span>{/if}
                  {#if matchedBecause(row)}<span class="because">because {matchedBecause(row)}</span>{/if}
                </div>
                {#if row.undoable || row.wrong_match_allowed}
                  <div class="lacts">
                    {#if row.undoable}<button class="tf-btn small" disabled={busy} onclick={() => undo(row)}>Undo</button>{/if}
                    {#if row.wrong_match_allowed}<button class="tf-btn small" disabled={busy} onclick={() => wrongMatch(row)}>Wrong match</button>{/if}
                  </div>
                {/if}
              </div>
            {/each}
          {/each}
        {/if}
      </section>

      <section class="block" id="rule-definition">
        <h2>Definition</h2>
        <p class="help">rule.toml as installed. Saving lapses any automatic approval: the files you approved are no longer these.</p>
        {#if detail.rule_toml === null || detail.rule_toml === undefined}
          <div class="warnline">rule.toml could not be read.</div>
        {:else}
          <textarea class="toml" spellcheck="false" aria-label="rule.toml" bind:value={editText}></textarea>
          {#if editError}<div class="warnline">{editError}</div>{/if}
          <div class="tf-bar" class:live={editText !== detail.rule_toml}>
            <span class="tf-count">{editText !== detail.rule_toml ? 'Unsaved changes' : 'No changes'}</span>
            <button class="tf-btn" disabled={editText === detail.rule_toml} onclick={() => (editText = detail.rule_toml)}>Discard</button>
            <button class="tf-btn primary" disabled={busy || editText === detail.rule_toml} onclick={saveEdit}>Save</button>
          </div>
        {/if}
      </section>
    {/if}
  {/if}
</div>

{#if approval}
  <div class="sheetwrap" role="presentation" onclick={(e) => e.target === e.currentTarget && !busy && (approval = null)}>
    <div class="sheet" role="dialog" aria-modal="true" aria-label="Approve">
      <h2>Raise {groupLabel(approval.group)} to {approval.mode}</h2>
      {#if approval.mode === 'auto'}
        <p class="help">The rule will act on its own on new mail. You are approving these files exactly; any later edit drops it back until you approve again.</p>
      {:else}
        <p class="help">Matches are offered in the mail desk for you to accept; nothing acts on its own.</p>
      {/if}
      {#if approval.error}<div class="warnline">{approval.error}</div>{/if}
      <div class="facts">
        {#each approvalBuckets as b}
          <span>{b.matched_actions} matched action{b.matched_actions === 1 ? '' : 's'} on {b.matching_threads} thread{b.matching_threads === 1 ? '' : 's'} under the current matcher ({verdictLabel(b.verdict_input)}){#if b.shadow_upper_bound} — shadow counts are an upper bound{/if}</span>
        {:else}
          <span>Nothing has matched under the current matcher yet.</span>
        {/each}
        <button class="linkish accent" onclick={() => (approval = null)}>close and read the ledger first</button>
      </div>
      {#if !approval.inputs}
        <div class="warnline">This rule's files could not be read, so there is nothing to approve. Fix the definition first.</div>
      {:else}
        {#each Object.entries(approval.inputs.files ?? {}) as [file, text]}
          <div class="filehead mono">{file}</div>
          <pre class="file">{text}</pre>
        {/each}
      {/if}
      <div class="tf-bar">
        <span class="tf-count"></span>
        <button class="tf-btn" disabled={busy} onclick={() => (approval = null)}>Cancel</button>
        <button class="tf-btn primary" disabled={busy || !approval.inputs?.digest} onclick={approve}>Approve these files</button>
      </div>
    </div>
  </div>
{/if}

{#if peek}
  <div class="sheetwrap" role="presentation" onclick={(e) => e.target === e.currentTarget && (peek = null)}>
    <div class="sheet" role="dialog" aria-modal="true" aria-label="Thread">
      <div class="sheettop">
        <h2 class="grow">{peekTitle}</h2>
        <button class="x" aria-label="Close" onclick={() => (peek = null)}>✕</button>
      </div>
      <!-- Third-party text: rendered as text nodes only, never markup. -->
      {#if peek.status === 'loading'}
        <div class="empty">reading the thread…</div>
      {:else if peek.status === 'error'}
        <div class="warnline">Could not read this thread: {peek.text}</div>
      {:else if parsedPeek}
        {#each parsedPeek.messages as m}
          <article class="peekmsg">
            <div class="msgmeta">{m.meta}</div>
            <MailBody text={m.body} />
          </article>
        {/each}
      {:else}
        <pre class="file">{peek.text}</pre>
      {/if}
    </div>
  </div>
{/if}

<style>
  .rulespage { flex: 1; overflow-y: auto; padding: 14px var(--gutter) 40px; max-width: 920px; width: 100%; margin: 0 auto; box-sizing: border-box; }
  /* The app's settings gear sits over the top-right corner on every view. */
  .head { display: flex; align-items: center; gap: 10px; margin-bottom: 12px; flex-wrap: wrap; padding-right: calc(var(--gutter-gear) - var(--gutter)); }
  .head h1 { margin: 0; font-size: 20px; font-weight: 600; color: var(--text); overflow-wrap: anywhere; }
  .grow { flex: 1; }
  .mono { font-family: var(--mono); }
  .backlink { background: none; border: none; color: var(--accent-300); font-size: 14px; padding: 8px 4px; min-height: 40px; cursor: pointer; }
  .linkish { background: none; border: none; padding: 0; color: inherit; text-align: left; cursor: pointer; font: inherit; }
  .linkish:hover .what, button.linkish:focus-visible { text-decoration: underline; }
  .empty, .help { color: var(--text-muted); font-size: 13px; line-height: 1.45; }
  .empty { padding: 18px 0; }
  .help { margin: 4px 0 10px; }
  .lede { color: var(--text); margin: 0 0 10px; }
  .warnline { background: color-mix(in srgb, var(--hazard) 14%, transparent); border: 1px solid color-mix(in srgb, var(--hazard) 45%, transparent); color: var(--text); border-radius: 8px; padding: 10px 12px; font-size: 13px; margin: 8px 0; overflow-wrap: anywhere; }
  .hold { font-size: 12px; color: var(--hazard); overflow-wrap: anywhere; }
  .heldword { color: var(--hazard); }
  .bad { color: var(--hazard); }
  .notice { display: flex; align-items: flex-start; gap: 10px; border-radius: 8px; padding: 10px 12px; margin: 0 0 12px; font-size: 13px; background: var(--accent-900); color: var(--text); }
  .notice.bad, .notice.held { background: color-mix(in srgb, var(--hazard) 16%, transparent); }
  .x { background: none; border: none; color: var(--text-muted); cursor: pointer; min-width: 32px; min-height: 32px; }

  .intro { background: var(--bg); border: 1px solid var(--accent-900); border-radius: 12px; padding: 16px; color: var(--text); font-size: 14px; line-height: 1.5; }
  .intro p { margin: 0 0 10px; }

  .cards { display: flex; flex-direction: column; gap: 10px; }
  .card { display: flex; align-items: stretch; background: var(--bg); border: 1px solid var(--accent-900); border-radius: 12px; overflow: hidden; }
  .cardmain { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px; padding: 12px 14px; background: none; border: none; text-align: left; color: var(--text); cursor: pointer; font: inherit; }
  .cardmain:hover { background: color-mix(in srgb, var(--accent-900) 40%, transparent); }
  .cardtop { display: flex; flex-wrap: wrap; align-items: baseline; gap: 4px 10px; }
  .rname { font-family: var(--mono); font-size: 14px; overflow-wrap: anywhere; }
  .desc { color: var(--text-muted); font-size: 13px; }
  .chips { display: flex; flex-wrap: wrap; gap: 6px; }
  .chip { display: inline-flex; align-items: center; font-family: var(--mono); font-size: 11px; padding: 3px 8px; border-radius: var(--radius-chip); border: 1px solid var(--accent-900); color: var(--text-muted); white-space: nowrap; }
  .chip.mode.auto { color: var(--void); background: var(--accent-400); border-color: var(--accent-400); }
  .chip.mode.propose { color: var(--text); border-color: var(--accent-500); }
  .chip.mode.off { opacity: 0.6; }
  .chip.mode.unknown, .chip.unknown { color: var(--hazard); border-color: var(--hazard); }
  .chip.done { color: var(--text); border-color: var(--accent-500); }
  .chip.bad, .chip.held { color: var(--hazard); border-color: color-mix(in srgb, var(--hazard) 60%, transparent); }
  .was { opacity: 0.75; }
  .facts { display: flex; flex-wrap: wrap; gap: 4px 14px; font-size: 12px; color: var(--text-muted); }
  .pausebtn { flex-shrink: 0; min-width: 72px; background: var(--surface); border: none; border-left: 1px solid var(--accent-900); color: var(--text); font-size: 13px; cursor: pointer; }
  .footer { display: flex; align-items: center; flex-wrap: wrap; gap: 10px; margin-top: 16px; font-size: 13px; color: var(--text); }

  .block { margin: 22px 0 0; }
  .block h2 { font-family: var(--mono); font-size: 11px; font-weight: 500; letter-spacing: 0.08em; text-transform: uppercase; color: var(--accent-300); margin: 0 0 4px; }
  .grouprow { display: flex; flex-direction: column; gap: 8px; padding: 12px 0; border-top: 1px solid var(--accent-900); }
  .glabel { display: flex; flex-direction: column; gap: 2px; }
  .gname { font-size: 14px; color: var(--text); }
  .gstate { font-size: 12px; color: var(--text-muted); }
  .gstate strong { color: var(--text); font-weight: 500; }
  .tf-seg { flex-wrap: wrap; max-width: 100%; }
  .tf-seg button { min-height: 40px; }
  .measure { display: flex; flex-wrap: wrap; gap: 2px 12px; font-size: 12px; color: var(--text-muted); }
  .mk { color: var(--text); }

  .testacct { margin: 12px 0; padding: 10px 12px; border: 1px solid var(--accent-900); border-radius: 10px; }
  .testhead { display: flex; flex-wrap: wrap; gap: 4px 12px; align-items: baseline; font-size: 13px; color: var(--text-muted); margin-bottom: 6px; }
  .testhead strong { color: var(--text); font-family: var(--mono); font-weight: 500; }

  .sechead { font-size: 12px; font-weight: 500; color: var(--text-muted); margin: 18px 0 4px; }
  .lrow { display: flex; flex-direction: column; gap: 4px; padding: 10px 0; border-top: 1px solid color-mix(in srgb, var(--accent-900) 70%, transparent); }
  .lrow.compact { flex-direction: row; flex-wrap: wrap; align-items: baseline; gap: 4px 10px; padding: 6px 0; }
  .lmain { display: flex; gap: 10px; align-items: baseline; min-width: 0; }
  .when { flex-shrink: 0; font-family: var(--mono); font-size: 11px; color: var(--text-muted); min-width: 52px; }
  .msg { min-width: 0; font-size: 13px; color: var(--text); overflow-wrap: anywhere; }
  .who { font-weight: 500; }
  .what { color: var(--text-muted); }
  .ldetail { display: flex; flex-wrap: wrap; gap: 4px 10px; align-items: center; font-size: 12px; color: var(--text-muted); padding-left: 62px; }
  .because { overflow-wrap: anywhere; }
  .note, .accent { color: var(--accent-300); }
  .lacts { display: flex; gap: 8px; padding-left: 62px; }
  .tf-btn.small { min-height: 36px; padding: 0 12px; font-size: 13px; }

  .toml { width: 100%; min-height: 220px; box-sizing: border-box; padding: 12px; background: var(--void); border: 1px solid var(--accent-900); border-radius: 8px; color: var(--text); font-family: var(--mono); font-size: 13px; line-height: 1.5; resize: vertical; }
  .toml:focus { outline: none; border-color: var(--accent-500); }
  .tf-seg.wrap { flex-wrap: wrap; }

  .sheetwrap { position: fixed; inset: 0; z-index: 50; background: color-mix(in srgb, var(--void) 70%, transparent); display: flex; align-items: flex-end; justify-content: center; }
  .sheet { width: 100%; max-width: 760px; max-height: 88vh; overflow-y: auto; box-sizing: border-box; background: var(--bg); border: 1px solid var(--accent-900); border-radius: 14px 14px 0 0; padding: 16px 20px 0; } /* not --gutter: wide, it centres to the measure */
  .sheet h2 { margin: 0 0 6px; font-size: 17px; color: var(--text); overflow-wrap: anywhere; }
  .sheettop { display: flex; align-items: flex-start; gap: 8px; }
  .filehead { font-size: 11px; color: var(--accent-300); margin: 14px 0 4px; }
  .file { margin: 0; padding: 12px; background: var(--void); border: 1px solid var(--accent-900); border-radius: 8px; font-family: var(--mono); font-size: 12px; line-height: 1.5; color: var(--text); white-space: pre-wrap; overflow-wrap: anywhere; }
  .peekmsg { padding: 12px 0; border-top: 1px solid var(--accent-900); }
  .msgmeta { font-family: var(--mono); font-size: 11px; color: var(--text-muted); margin-bottom: 6px; overflow-wrap: anywhere; }
  @media (min-width: 900px) {
    .sheetwrap { align-items: center; }
    .sheet { border-radius: 14px; padding-bottom: 0; }
  }
</style>
