<script>
  // The Edit button's modal: paint (or box) the part of a picture to change,
  // say what should change, send. Painting dims the rest of the picture —
  // what stays bright is what will be redrawn; the dim part comes back pixel
  // for pixel (`image_generate`'s mask, IMAGE-REGION-EDIT-RESEARCH.md §5).
  // Nothing painted edits the whole picture, as the button did before.
  import { toNatural, boxFrom, defaultBrush, hasPaint } from './image-edit.js';

  let { src, path, initial = '', busy = false, error = null, onsend, onclose } = $props();

  let img = $state(null);
  let view = $state(null);
  // Only the draft at opening becomes the instruction; later edits to the
  // chat's draft must not overwrite what is being typed here.
  // svelte-ignore state_referenced_locally
  let words = $state(initial);
  let natural = $state(null);
  let tool = $state('brush');
  let size = $state(24);
  let ops = $state([]);
  let textarea = $state(null);

  // What is painted, at the picture's own size: white where it will change,
  // transparent elsewhere. `base` holds the committed operations; a stroke
  // draws into it as it goes, a box only when it is let go.
  let base = null;
  let live = null; // the operation under the pointer
  let start = null;

  const painted = $derived(hasPaint(ops));
  const canSend = $derived(!busy && words.trim().length > 0);
  const dirty = $derived(ops.length > 0 || words.trim() !== initial.trim());

  function loaded() {
    natural = { width: img.naturalWidth, height: img.naturalHeight };
    size = defaultBrush(natural.width);
    base = document.createElement('canvas');
    base.width = view.width = natural.width;
    base.height = view.height = natural.height;
    render();
  }

  function draw(ctx, op) {
    ctx.save();
    if (op.kind === 'box') {
      ctx.fillStyle = '#fff';
      ctx.fillRect(op.x, op.y, op.w, op.h);
    } else {
      ctx.globalCompositeOperation = op.erase ? 'destination-out' : 'source-over';
      ctx.strokeStyle = ctx.fillStyle = '#fff';
      ctx.lineWidth = op.size;
      ctx.lineCap = ctx.lineJoin = 'round';
      const [first, ...rest] = op.points;
      if (!rest.length) {
        ctx.beginPath();
        ctx.arc(first[0], first[1], op.size / 2, 0, Math.PI * 2);
        ctx.fill();
      } else {
        ctx.beginPath();
        ctx.moveTo(...first);
        for (const p of rest) ctx.lineTo(...p);
        ctx.stroke();
      }
    }
    ctx.restore();
  }

  function replay() {
    const ctx = base.getContext('2d');
    ctx.clearRect(0, 0, base.width, base.height);
    for (const op of ops) draw(ctx, op);
  }

  // The view: the picture dimmed everywhere but what is painted, which also
  // takes a faint wash of the accent so a pale area still reads as chosen.
  function render() {
    if (!view || !base) return;
    const ctx = view.getContext('2d');
    const { width: w, height: h } = view;
    ctx.clearRect(0, 0, w, h);
    const any = hasPaint(ops) || (live && (live.kind === 'box' || !live.erase));
    if (!any) return;
    let mask = base;
    if (live?.kind === 'box') {
      mask = document.createElement('canvas');
      mask.width = w;
      mask.height = h;
      const m = mask.getContext('2d');
      m.drawImage(base, 0, 0);
      draw(m, live);
    }
    ctx.fillStyle = 'rgba(18, 20, 31, 0.62)';
    ctx.fillRect(0, 0, w, h);
    ctx.globalCompositeOperation = 'destination-out';
    ctx.drawImage(mask, 0, 0);
    ctx.globalCompositeOperation = 'source-over';
    const tint = document.createElement('canvas');
    tint.width = w;
    tint.height = h;
    const t = tint.getContext('2d');
    t.fillStyle = 'rgb(181, 171, 252)';
    t.fillRect(0, 0, w, h);
    t.globalCompositeOperation = 'destination-in';
    t.drawImage(mask, 0, 0);
    ctx.globalAlpha = 0.18;
    ctx.drawImage(tint, 0, 0);
    ctx.globalAlpha = 1;
  }

  function point(e) {
    return toNatural(e.clientX, e.clientY, view.getBoundingClientRect(), natural);
  }

  function down(e) {
    if (!natural || busy || e.button > 0) return;
    view.setPointerCapture(e.pointerId);
    start = point(e);
    if (tool === 'box') {
      live = { kind: 'box', ...boxFrom(start, start) };
    } else {
      live = { kind: 'stroke', erase: tool === 'erase', size, points: [start] };
      draw(base.getContext('2d'), live);
    }
    render();
  }

  function move(e) {
    if (!live) return;
    const p = point(e);
    if (live.kind === 'box') {
      live = { kind: 'box', ...boxFrom(start, p) };
    } else {
      const last = live.points[live.points.length - 1];
      live.points.push(p);
      draw(base.getContext('2d'), { ...live, points: [last, p] });
    }
    render();
  }

  function up() {
    if (!live) return;
    const op = live;
    live = null;
    if (op.kind === 'box' && (op.w < 2 || op.h < 2)) {
      render();
      return;
    }
    ops = [...ops, op];
    if (op.kind === 'box') draw(base.getContext('2d'), op);
    render();
  }

  function undo() {
    if (!ops.length || busy) return;
    ops = ops.slice(0, -1);
    replay();
    render();
  }

  function clear() {
    if (busy) return;
    ops = [];
    replay();
    render();
  }

  // The mask the server reads: the picture's own size, white where painted,
  // black everywhere else.
  function maskBlob() {
    const out = document.createElement('canvas');
    out.width = base.width;
    out.height = base.height;
    const ctx = out.getContext('2d');
    ctx.fillStyle = '#000';
    ctx.fillRect(0, 0, out.width, out.height);
    ctx.drawImage(base, 0, 0);
    return new Promise((resolve) => out.toBlob(resolve, 'image/png'));
  }

  async function send() {
    if (!canSend) return;
    const mask = painted ? await maskBlob() : null;
    onsend?.({ text: words, mask });
  }

  function close() {
    if (!busy) onclose?.();
  }

  function keydown(e) {
    if (e.key === 'Escape') {
      e.preventDefault();
      close();
    } else if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      send();
    } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'z' && e.target !== textarea) {
      e.preventDefault();
      undo();
    }
  }

  $effect(() => {
    // A precise pointer means a keyboard to type with; on a phone, focusing
    // the text would slide the keyboard over the picture being painted.
    if (textarea && globalThis.matchMedia?.('(pointer: fine)').matches) textarea.focus();
  });
</script>

<svelte:window onkeydown={keydown} />

<!-- A click outside closes only an untouched modal: paint is work. -->
<div class="editscrim" onclick={() => !dirty && close()} aria-hidden="true"></div>
<div class="editsheet" role="dialog" aria-modal="true" aria-labelledby="edit-title">
  <header class="edithead">
    <h2 id="edit-title">Edit picture</h2>
    <p class="edithint">
      {#if painted}
        Only the bright area will change. The dimmed part stays exactly as it is.
      {:else}
        Paint over what should change, or leave it unpainted to edit the whole picture.
      {/if}
    </p>
  </header>

  <div class="stage">
    <img bind:this={img} {src} alt={`The picture being edited, ${path}`} onload={loaded} draggable="false" />
    <canvas
      bind:this={view}
      class:boxing={tool === 'box'}
      onpointerdown={down}
      onpointermove={move}
      onpointerup={up}
      onpointercancel={up}
      aria-label="Paint the area to change"
    ></canvas>
  </div>

  <div class="tools" role="toolbar" aria-label="Painting tools">
    <div class="seg" role="group" aria-label="Tool">
      <button type="button" aria-pressed={tool === 'brush'} onclick={() => (tool = 'brush')}>Brush</button>
      <button type="button" aria-pressed={tool === 'box'} onclick={() => (tool = 'box')}>Box</button>
      <button type="button" aria-pressed={tool === 'erase'} onclick={() => (tool = 'erase')}>Eraser</button>
    </div>
    {#if tool !== 'box' && natural}
      <label class="size">
        <span>Size</span>
        <input type="range" min="4" max={Math.max(40, Math.round(natural.width / 6))} bind:value={size} />
      </label>
    {/if}
    <span class="grow"></span>
    <button type="button" class="quiet" onclick={undo} disabled={!ops.length || busy}>Undo</button>
    <button type="button" class="quiet" onclick={clear} disabled={!ops.length || busy}>Clear</button>
  </div>

  <label class="words">
    <span class="sr">What should change</span>
    <textarea
      bind:this={textarea}
      bind:value={words}
      rows="2"
      placeholder={painted ? 'What should change here? e.g. make the dress green' : 'What should change? e.g. make it dusk'}
      disabled={busy}
    ></textarea>
  </label>

  {#if error}
    <p class="editerr" role="alert">{error}</p>
  {/if}

  <footer class="editfoot">
    <button type="button" class="quiet" onclick={close} disabled={busy}>Cancel</button>
    <button type="button" class="go" onclick={send} disabled={!canSend}>
      {busy ? 'Sending…' : painted ? 'Edit painted area' : 'Edit whole picture'}
    </button>
  </footer>
</div>

<style>
  .editscrim {
    position: fixed;
    inset: 0;
    background: rgba(18, 20, 31, 0.72);
    z-index: 60;
  }
  .editsheet {
    position: fixed;
    z-index: 61;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: min(960px, calc(100vw - 32px));
    max-height: calc(100dvh - 32px);
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 18px 20px 16px;
    background: var(--bg);
    border: 1px solid var(--accent-900);
    border-radius: 12px;
    color: var(--text);
    font-family: var(--sans);
  }
  .edithead h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 600;
  }
  .edithint {
    margin: 4px 0 0;
    font-size: 13px;
    line-height: 1.45;
    color: var(--text-muted);
  }
  .stage {
    position: relative;
    width: fit-content;
    max-width: 100%;
    margin: 0 auto;
    border-radius: var(--radius);
    overflow: hidden;
    background: var(--void);
  }
  .stage img {
    display: block;
    max-width: 100%;
    max-height: 58vh;
    user-select: none;
    -webkit-user-drag: none;
  }
  .stage canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    touch-action: none;
    cursor: crosshair;
  }
  .stage canvas.boxing {
    cursor: cell;
  }
  .tools {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 12px;
  }
  .seg {
    display: inline-flex;
    border: 1px solid var(--accent-700);
    border-radius: 999px;
    overflow: hidden;
  }
  .seg button {
    padding: 6px 14px;
    font: 500 13px var(--sans);
    color: var(--text-muted);
    background: none;
    border: 0;
    cursor: pointer;
  }
  .seg button + button {
    border-left: 1px solid var(--accent-700);
  }
  .seg button[aria-pressed='true'] {
    color: var(--void);
    background: var(--accent-400);
  }
  .size {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--text-muted);
  }
  .size input {
    width: 120px;
    accent-color: var(--accent-400);
  }
  .grow {
    flex: 1;
  }
  .quiet,
  .go {
    padding: 7px 16px;
    font: 500 14px var(--sans);
    border-radius: 999px;
    cursor: pointer;
  }
  .quiet {
    color: var(--accent-300);
    background: none;
    border: 1px solid var(--accent-700);
  }
  .go {
    color: var(--void);
    background: var(--accent-400);
    border: 1px solid var(--accent-400);
  }
  .quiet:disabled,
  .go:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .seg button:focus-visible,
  .quiet:focus-visible,
  .go:focus-visible,
  .words textarea:focus-visible {
    outline: 2px solid var(--accent-300);
    outline-offset: 2px;
  }
  .words textarea {
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    padding: 10px 12px;
    font: 15px/1.45 var(--sans);
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--accent-700);
    border-radius: var(--radius);
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  .editerr {
    margin: 0;
    font-size: 13px;
    color: var(--hazard);
  }
  .editfoot {
    display: flex;
    justify-content: flex-end;
    gap: 10px;
  }
  /* A phone: the whole screen, the picture as tall as the keyboard allows. */
  @media (max-width: 640px) {
    .editsheet {
      top: 0;
      left: 0;
      transform: none;
      width: 100vw;
      height: 100dvh;
      max-height: 100dvh;
      border-radius: 0;
      border: 0;
      padding: 14px 16px calc(12px + env(safe-area-inset-bottom));
    }
    .stage img {
      max-height: 48dvh;
    }
    /* The send button under the thumb, not mid-screen. */
    .editfoot {
      margin-top: auto;
    }
    .editfoot .go {
      flex: 1;
    }
  }
</style>
