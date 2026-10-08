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
  import { moveTo, dropAt } from './picture.js';

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
  // The dragged row is held by its id, never its index: a `queue` event
  // mid-drag — the running picture ending, which promotes the first waiting
  // one — shifts every index, and an index would move a different row than
  // the one under the finger (review of #607). `dragTo` is a gap: above row
  // `dragTo`, or below the last when it equals the line's length.
  let dragFrom = $state(null);
  let dragTo = $state(-1);
  let rows = $state([]);
  const ids = $derived(waiting.map((w) => w.call_id));
  // A row that left the line mid-drag (stopped, or promoted to running) ends
  // the drag, or its marker would stay drawn (review of #607).
  $effect(() => {
    if (dragFrom !== null && !ids.includes(dragFrom)) {
      dragFrom = null;
      dragTo = -1;
    }
  });

  function start(e, i) {
    dragFrom = waiting[i]?.call_id ?? null;
    dragTo = i;
    e.currentTarget.setPointerCapture(e.pointerId);
    e.preventDefault();
  }
  function move(e) {
    if (dragFrom === null) return;
    // The gap above the first row whose middle the pointer is above, or the
    // gap below the last.
    let to = waiting.length;
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
    if (dragFrom !== null && dragTo >= 0) {
      const order = dropAt(ids, dragFrom, dragTo);
      if (order) onreorder(order);
    }
    dragFrom = null;
    dragTo = -1;
  }
  function step(i, by) {
    const j = i + by;
    if (j < 0 || j >= waiting.length) return;
    const order = moveTo(ids, waiting[i].call_id, j);
    if (order) onreorder(order);
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
      <div
        class="pqrow"
        class:dragging={dragFrom === item.call_id}
        class:target={dragFrom !== null && dragTo === i}
        class:targetbelow={dragFrom !== null && dragTo === waiting.length && i === waiting.length - 1}
        bind:this={rows[i]}
      >
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
  .pqrow.targetbelow {
    box-shadow: inset 0 -2px 0 var(--accent-400);
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
