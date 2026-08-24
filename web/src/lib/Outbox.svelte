<script>
  // The outbox on the phone. The page renders the whole reviewable object —
  // taint warning, headers, prose, everything-else, and the quoted source
  // the draft answers — because approving without reading is the failure
  // this queue exists to prevent. Every action drives a `mecha outbox …`
  // verb on the box; the confirm sheet is what earns `--yes`.
  let pending = $state([]);
  let resolved = $state(0);
  let detail = $state(null);
  let error = $state(null);
  let confirming = $state(false);
  let rejecting = $state(false);
  let editing = $state(false);
  let editDraft = $state('');
  let rejectReason = $state('');
  let busy = $state(false);

  async function loadList() {
    try {
      const res = await fetch('/api/outbox');
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${(await res.text()).trim()}`);
      const data = await res.json();
      pending = data.pending;
      resolved = data.resolved;
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  async function open(id) {
    try {
      const res = await fetch(`/api/outbox/${id}`);
      if (!res.ok) throw new Error((await res.text()).trim());
      detail = await res.json();
      confirming = rejecting = editing = false;
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }

  function back() {
    detail = null;
    confirming = rejecting = editing = false;
    loadList();
  }

  async function act(path, body) {
    busy = true;
    try {
      const res = await fetch(`/api/outbox/${detail.id}/${path}`, {
        method: 'POST',
        headers: body ? { 'content-type': 'application/json' } : {},
        body: body ? JSON.stringify(body) : undefined,
      });
      const text = await res.text();
      if (!res.ok) throw new Error(text.trim());
      return true;
    } catch (e) {
      error = String(e?.message ?? e);
      return false;
    } finally {
      busy = false;
    }
  }

  async function approve() {
    if (await act('approve')) back();
  }
  async function reject() {
    if (!rejectReason.trim()) return;
    if (await act('reject', { reason: rejectReason.trim() })) {
      rejectReason = '';
      back();
    }
  }
  async function saveEdit() {
    if (await act('edit', { body: editDraft })) {
      editing = false;
      open(detail.id);
    }
  }

  loadList();
  const timer = setInterval(() => {
    if (!detail) loadList();
  }, 30_000);
  $effect(() => () => clearInterval(timer));

  const age = (iso) => {
    const days = Math.floor((Date.now() - Date.parse(iso)) / 86_400_000);
    if (days > 0) return `${days}d ago`;
    const hours = Math.floor((Date.now() - Date.parse(iso)) / 3_600_000);
    return hours > 0 ? `${hours}h ago` : 'just now';
  };
</script>

{#snippet hazardGlyph(size = 13)}
  <svg viewBox="0 0 24 24" width={size} height={size} style="flex-shrink: 0" fill="none" stroke="var(--hazard)" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
    <path d="M12 4l9 16H3z" /><path d="M12 11v4M12 17.5v.5" />
  </svg>
{/snippet}

<div class="page">
  {#if !detail}
    <header>
      <span class="title">Outbox</span>
      <span class="chip">{pending.length} pending · {resolved} resolved</span>
    </header>
    <div class="scroll">
      {#if error}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}
      {#each pending as item}
        <button class="card rowbtn" onclick={() => open(item.id)}>
          <div class="rowtop">
            <span class="tool">{item.tool}</span>
            {#if item.tainted}{@render hazardGlyph(12)}{/if}
            <span class="when">{age(item.created_at)}</span>
          </div>
          <div class="summary">{item.summary}</div>
          {#if item.edited}<span class="edited">edited</span>{/if}
        </button>
      {:else}
        <div class="empty">Nothing waiting on you.</div>
      {/each}
    </div>
  {:else}
    <header>
      <button class="backbtn" onclick={back} aria-label="back">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M15 6l-6 6 6 6" /></svg>
      </button>
      <span class="title">Draft</span>
      <span class="chip">{detail.tool}</span>
    </header>
    <div class="scroll">
      {#if error}<div class="warnline">{@render hazardGlyph()}<span>{error}</span></div>{/if}

      {#if detail.taint.armed}
        <div class="card taintbanner">
          {@render hazardGlyph(16)}
          <div>
            <div class="taint-head">Drafted with untrusted content in context</div>
            <div class="taint-sub">If anything in this draft was not yours, an attacker may have put it there. Read all of it.</div>
          </div>
        </div>
      {/if}

      <div class="card headers">
        {#each detail.headers as [key, value]}
          <div class="hrow"><span class="hkey">{key}</span><span class="hval">{value}</span></div>
        {/each}
      </div>

      {#if editing}
        <textarea class="editbox" bind:value={editDraft} rows="10"></textarea>
        <div class="btnrow">
          <button class="btn" onclick={() => (editing = false)}>Discard</button>
          <button class="btn primary" disabled={busy} onclick={saveEdit}>Save edit</button>
        </div>
      {:else if detail.body}
        <div class="card prose">{detail.body}</div>
      {/if}

      {#if detail.other?.length}
        <div class="card headers">
          {#each detail.other as [key, value]}
            <div class="hrow"><span class="hkey">{key}</span><span class="hval">{value}</span></div>
          {/each}
        </div>
      {/if}

      {#each detail.sources as source}
        <div class="source">
          <div class="source-head">
            <span class="kicker">in reply to</span>
            <span class="source-tool">{source.tool} · {source.keys.join(', ')}</span>
          </div>
          <div class="quoted"><span class="gutter"></span><div class="qtext">{source.text}</div></div>
        </div>
      {/each}

      <div class="provenance">
        staged {age(detail.created_at)}{detail.session_id ? ` · session ${detail.session_id}` : ''}{detail.edited ? ' · edited' : ''}
      </div>

      {#if !editing}
        <div class="btnrow">
          <button
            class="btn"
            disabled={busy || detail.kind === 'publish'}
            onclick={() => {
              editDraft = detail.body ?? '';
              editing = true;
            }}
          >Edit prose</button>
          <button class="btn" disabled={busy} onclick={() => (rejecting = true)}>Reject…</button>
        </div>
        <button class="btn primary tall" disabled={busy} onclick={() => (confirming = true)}>
          Approve — confirms first
        </button>
      {/if}
    </div>

    {#if confirming}
      <div class="sheet">
        <div class="sheet-grip"></div>
        {#if detail.taint.armed}
          <div class="warnline">{@render hazardGlyph()}<span>This draft was written while the trifecta was armed. The exact arguments:</span></div>
          <pre class="argdump">{JSON.stringify(detail.args, null, 2)}</pre>
        {:else}
          <div class="sheet-text">Approve and execute <span class="tool">{detail.tool}</span>?</div>
          <div class="sheet-sub">{detail.summary}</div>
        {/if}
        <div class="btnrow">
          <button class="btn" onclick={() => (confirming = false)}>Back</button>
          <button class="btn primary" disabled={busy} onclick={approve}>
            {busy ? 'sending…' : 'Send it'}
          </button>
        </div>
      </div>
    {/if}

    {#if rejecting}
      <div class="sheet">
        <div class="sheet-grip"></div>
        <div class="sheet-text">Why? The reason is recorded on the item.</div>
        <textarea class="editbox" rows="3" bind:value={rejectReason} placeholder="wrong tone — too formal for Priya"></textarea>
        <div class="btnrow">
          <button class="btn" onclick={() => (rejecting = false)}>Back</button>
          <button class="btn primary" disabled={busy || !rejectReason.trim()} onclick={reject}>Reject</button>
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .page { flex: 1; display: flex; flex-direction: column; min-height: 0; position: relative; }
  header { display: flex; align-items: center; gap: 10px; padding: 22px 20px 12px; border-bottom: 1px solid var(--accent-900); }
  header .title { font-weight: 500; font-size: 17px; letter-spacing: -0.02em; flex: 1; }
  .backbtn { background: none; border: none; color: var(--text-muted); min-width: 44px; min-height: 44px; margin: -12px 0 -12px -12px; cursor: pointer; display: flex; align-items: center; justify-content: center; }
  .scroll { flex: 1; overflow-y: auto; padding: 14px 20px; display: flex; flex-direction: column; gap: 10px; }
  .rowbtn { text-align: left; padding: 14px; display: flex; flex-direction: column; gap: 7px; cursor: pointer; color: var(--text); font: inherit; }
  .rowtop { display: flex; align-items: center; gap: 8px; }
  .tool { font-family: var(--mono); font-size: 12px; color: var(--accent-400); }
  .when { font-family: var(--mono); font-size: 11px; color: var(--text-muted); margin-left: auto; }
  .summary { font-size: 14px; line-height: 1.4; }
  .edited { font-family: var(--mono); font-size: 10px; color: var(--text-muted); }
  .empty { color: var(--text-muted); font-size: 14px; padding: 24px 0; text-align: center; }
  .warnline { display: flex; align-items: flex-start; gap: 8px; font-size: 12px; color: var(--hazard); line-height: 1.45; }
  .taintbanner { display: flex; gap: 10px; padding: 12px 14px; }
  .taint-head { font-size: 13px; font-weight: 500; color: var(--hazard); }
  .taint-sub { font-size: 12px; color: var(--text-muted); line-height: 1.45; margin-top: 3px; }
  .headers { padding: 12px 14px; display: flex; flex-direction: column; gap: 7px; }
  .hrow { display: flex; gap: 10px; font-size: 13px; }
  .hkey { font-family: var(--mono); font-size: 11px; color: var(--accent-700); min-width: 58px; padding-top: 1px; }
  .hval { overflow-wrap: anywhere; }
  .prose { padding: 16px; font-size: 14px; line-height: 1.55; white-space: pre-wrap; }
  .source-head { display: flex; align-items: center; gap: 8px; margin-bottom: 8px; }
  .source-tool { font-family: var(--mono); font-size: 11px; color: var(--accent-700); }
  .quoted { display: flex; gap: 10px; }
  .gutter { width: 2px; background: var(--hazard); flex-shrink: 0; }
  .qtext { font-size: 13px; line-height: 1.55; color: var(--text-muted); white-space: pre-wrap; overflow-wrap: anywhere; }
  .provenance { font-family: var(--mono); font-size: 10px; color: var(--accent-700); }
  .btnrow { display: flex; gap: 10px; }
  .btn { flex: 1; min-height: 48px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: var(--radius); color: var(--text); font-size: 14px; cursor: pointer; }
  .btn.primary { background: var(--accent-400); color: var(--void); font-weight: 500; border: none; }
  .btn.tall { flex: none; width: 100%; }
  .btn:disabled { opacity: 0.5; cursor: default; }
  .editbox { width: 100%; background: var(--surface); border: 1px solid var(--accent-700); border-radius: var(--radius); color: var(--text); font-family: var(--sans); font-size: 15px; line-height: 1.5; padding: 12px 14px; resize: vertical; box-sizing: border-box; }
  .sheet { position: absolute; left: 0; right: 0; bottom: 0; background: var(--bg); border-top: 1px solid var(--accent-500); border-radius: 16px 16px 0 0; padding: 14px 20px 28px; display: flex; flex-direction: column; gap: 12px; }
  .sheet-grip { width: 36px; height: 4px; border-radius: 2px; background: var(--accent-900); align-self: center; }
  .sheet-text { font-size: 15px; font-weight: 500; }
  .sheet-sub { font-size: 13px; color: var(--text-muted); }
  .argdump { background: var(--void); border: 1px solid var(--accent-900); border-radius: var(--radius); padding: 12px; font-family: var(--mono); font-size: 11px; line-height: 1.5; overflow-x: auto; max-height: 240px; margin: 0; color: var(--text); }
</style>
