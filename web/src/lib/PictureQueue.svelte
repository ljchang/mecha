<script>
  // A chat's line of background jobs (`jobs::JobQueue`): the one running, with
  // its clock and its Stop, then the ones waiting in the order they will run,
  // each stoppable on its own and movable — by dragging its handle (pointer
  // events, so a finger works where HTML5 drag does not) or by ↑/↓. The one
  // running never moves: a render cannot be set aside. Shown only while two
  // or more are out; one alone is its own row in the chat (owner, 2026-10-08:
  // "see jobs in the queue … cancel individual ones … drag and drop the
  // order").
  //
  // `items`: `QueueItem`s as the server sends them (`queue` events and the
  // transcript's `queue`). `oncancel(callId)` stops one; `onreorder(ids)`
  // asks for the waiting ones in a new order. Neither changes `items`
  // itself: the server's next `queue` event does, so the panel never shows
  // a line the server refused.
  let { items = [], oncancel = () => {}, onreorder = () => {} } = $props();

  // The running one's clock, from what the server said it had run plus this
  // page's own clock since (a duration, never a timestamp).
  let now = $state(Date.now());
  let seenAt = $state(Date.now());
  $effect(() => {
    void items;
    seenAt = Date.now();
  });
  $effect(() => {
    const tick = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(tick);
  });
  const clock = (ms) => {
    const s = Math.max(0, Math.floor(ms / 1000));
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
  };

  const running = $derived(items.find((i) => i.running) ?? null);
  const waiting = $derived(items.filter((i) => !i.running));

  // A drag in progress: which row, and where it would land. The line stays
  // still while a finger moves — the target row is marked — so the rows the
  // pointer is measured against never shift under it; `onreorder` asks the
  // server for the new line on release.
  let dragFrom = $state(-1);
  let dragTo = $state(-1);
  let rows = $state([]);
  const moved = (from, to) => {
    const out = waiting.map((w) => w.call_id);
    const [id] = out.splice(from, 1);
    out.splice(to, 0, id);
    return out;
  };

  function start(e, i) {
    dragFrom = i;
    dragTo = i;
    e.currentTarget.setPointerCapture(e.pointerId);
    e.preventDefault();
  }
  function move(e) {
    if (dragFrom < 0) return;
    // The row whose middle the pointer is above, or the last one.
    let to = waiting.length - 1;
    for (let i = 0; i < rows.length; i++) {
      const r = rows[i]?.getBoundingClientRect();
      if (r && e.clientY < r.top + r.height / 2) {
        to = i;
        break;
      }
    }
    dragTo = to;
  }
  function end() {
    if (dragFrom >= 0 && dragTo >= 0 && dragFrom !== dragTo) {
      onreorder(moved(dragFrom, dragTo));
    }
    dragFrom = -1;
    dragTo = -1;
  }
  function step(i, by) {
    const j = i + by;
    if (j < 0 || j >= waiting.length) return;
    onreorder(moved(i, j));
  }
</script>

{#if items.length > 1}
  <div class="pq" role="region" aria-label="pictures in line">
    <div class="pqhead">In line: {items.length}</div>
    {#if running}
      <div class="pqrow on">
        <span class="pqstate">drawing</span>
        <span class="pqlabel" title={running.label}>{running.label || running.tool}</span>
        <span class="pqclock">{clock((running.elapsed_ms ?? 0) + (now - seenAt))}</span>
        <button class="pqbtn stop" onclick={() => oncancel(running.call_id)} aria-label="stop this picture">Stop</button>
      </div>
    {/if}
    {#each waiting as item, i (item.call_id)}
      <div class="pqrow" class:dragging={dragFrom === i} class:target={dragFrom >= 0 && dragTo === i && dragTo !== dragFrom} bind:this={rows[i]}>
        <span
          class="pqhandle"
          role="button"
          tabindex="-1"
          aria-label="drag to reorder"
          onpointerdown={(e) => start(e, i)}
          onpointermove={move}
          onpointerup={end}
          onpointercancel={end}
        >⠿</span>
        <span class="pqstate">{i + 1}</span>
        <span class="pqlabel" title={item.label}>{item.label || item.tool}</span>
        <button class="pqbtn" disabled={i === 0} onclick={() => step(i, -1)} aria-label="move up">↑</button>
        <button class="pqbtn" disabled={i === waiting.length - 1} onclick={() => step(i, 1)} aria-label="move down">↓</button>
        <button class="pqbtn stop" onclick={() => oncancel(item.call_id)} aria-label="remove from the line">✕</button>
      </div>
    {/each}
  </div>
{/if}

<style>
  .pq {
    margin: 6px 0;
    padding: 8px 10px;
    border: 1px solid var(--accent-900);
    border-radius: 10px;
    font-size: 13px;
  }
  .pqhead {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-muted);
    margin-bottom: 4px;
  }
  .pqrow {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 0;
    min-height: 32px;
  }
  .pqrow.on .pqstate {
    color: var(--accent-400);
  }
  .pqrow.dragging {
    opacity: 0.6;
  }
  .pqrow.target {
    box-shadow: inset 0 2px 0 var(--accent-400);
  }
  .pqstate {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-muted);
    min-width: 3.5em;
  }
  .pqlabel {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pqclock {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-muted);
  }
  .pqhandle {
    cursor: grab;
    touch-action: none;
    user-select: none;
    padding: 4px 6px;
    color: var(--text-muted);
  }
  .pqbtn {
    padding: 4px 10px;
    min-width: 32px;
    min-height: 28px;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--accent-400);
    background: none;
    border: 1px solid var(--accent-900);
    border-radius: 999px;
    cursor: pointer;
  }
  .pqbtn:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .pqbtn.stop {
    color: var(--hazard);
    border-color: var(--hazard);
  }
</style>
