<script>
  // The chat view: a rendering of the conversation the server owns, plus a
  // live SSE feed of the run in flight. Sending during a run steers it —
  // the server folds the text into the tool-results turn.
  let entries = $state([]);
  let streaming = $state('');
  let running = $state(false);
  let taint = $state(null);
  let usage = $state(null);
  let model = $state('');
  let draft = $state('');
  let error = $state(null);
  let transcriptEl = $state(null);

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
      const res = await fetch('/api/chat');
      if (!res.ok) throw new Error(`HTTP ${res.status}: ${(await res.text()).trim()}`);
      const data = await res.json();
      entries = data.entries.map((e) =>
        e.kind === 'tool' ? { ...e, pending: false } : e
      );
      running = data.running;
      taint = data.taint;
      model = data.model;
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
    const source = new EventSource('/api/chat/events');
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

  load();
  const source = subscribe();
  $effect(() => () => source.close());

  async function send() {
    const text = draft.trim();
    if (!text) return;
    draft = '';
    try {
      const res = await fetch('/api/chat/send', {
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

  async function cancel() {
    try {
      await fetch('/api/chat/cancel', { method: 'POST' });
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
    <span class="title">Chat</span>
    <div class="meta">
      {#if taint?.untrusted || taint?.private}
        <span
          class="chip taint"
          title="what this conversation has touched decides what it may still do"
        >
          {taint.private ? 'private' : ''}{taint.private && taint.untrusted ? ' + ' : ''}{taint.untrusted ? 'untrusted' : ''}
        </span>
      {/if}
      <span class="chip">{model || '…'}</span>
    </div>
  </header>

  <div class="transcript" bind:this={transcriptEl}>
    {#if error}
      <div class="notice">{error}</div>
    {/if}
    {#each entries as entry}
      {#if entry.kind === 'user'}
        <div class="bubble" class:queued={entry.queued}>
          {entry.text}
          {#if entry.queued}<span class="queued-tag">steered</span>{/if}
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
    <div class="input-row">
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
    padding: 22px 20px 12px;
    border-bottom: 1px solid var(--accent-900);
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
</style>
