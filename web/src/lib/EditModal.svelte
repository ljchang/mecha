<script>
  // The Edit button's modal: paint (or box) the part of a picture to change,
  // say what should change, send. Painting dims the rest of the picture —
  // what stays bright is what will be redrawn; the dim part comes back pixel
  // for pixel (`image_generate`'s mask, IMAGE-REGION-EDIT-RESEARCH.md §5).
  // Nothing painted edits the whole picture, as the button did before.
  //
  // With `multi` (the persona chat, IMAGE-REGION-EDIT-RESEARCH.md §7, M2) the
  // owner paints up to four regions, each in its own colour with its own
  // words; the send carries a colour-indexed picture of them and the words
  // per colour, and the harness builds the legend the model reads.
  import {
    toNatural,
    boxFrom,
    defaultBrush,
    hasPaint,
    pickColours,
    indexPixels,
    MAX_REGIONS,
    editDirty,
    firstRegionWords,
    opsWithout,
    workSize,
  } from './image-edit.js';

  let { src, path, initial = '', busy = false, error = null, multi = false, onsend, onclose } = $props();

  // The regions: colour and words each, in the order added. Single mode has
  // one, uncoloured. `layers` holds each region's painted pixels.
  let palette = [];
  let regionList = $state([]);
  let active = $state(0);
  let layers = [];
  // Which regions hold paint, read from their layers like `painted`.
  let paintedRegions = $state([]);

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
  // Set before the mask is encoded, so a second click in that window cannot
  // send twice (review of #429).
  let encoding = $state(false);
  let encodeError = $state(null);

  // What is painted, at the picture's own size: white where it will change,
  // transparent elsewhere. `base` holds the committed operations; a stroke
  // draws into it as it goes, a box only when it is let go.
  let base = null;
  // Scratch canvases for `render`, made once at load: a full-size canvas per
  // pointer event is 50–100 MB of churn on a phone photo (review of #429).
  let preview = null;
  let tint = null;
  let live = null; // the operation under the pointer
  let start = null;

  // Whether anything is painted, read from the canvas itself once a stroke
  // or box is let go: the op list cannot tell a stroke erased away from one
  // that is still there (review of #429). `hasPaint` answers first, cheaply.
  let painted = $state(false);
  function repaint() {
    // Only regions mode reads which regions hold paint: in single mode it
    // would be a second full-size readback per stroke (review of #623).
    paintedRegions = multi
      ? layers.map((l, k) => hasPaint(ops.filter((op) => op.region === k)) && layerHasPaint(l))
      : [];
    const was = painted;
    if (!base || !hasPaint(ops)) {
      painted = false;
      return;
    }
    const data = base.getContext('2d').getImageData(0, 0, base.width, base.height).data;
    let any = false;
    for (let i = 3; i < data.length; i += 4) {
      if (data[i] > 127) {
        any = true;
        break;
      }
    }
    painted = any;
    // The whole-picture box gives way to the regions' own boxes: what was
    // typed in it carries into the first region that holds paint, which is
    // not region one when a region was added before the first stroke.
    if (multi && !was && painted) {
      const first = paintedRegions.findIndex(Boolean);
      if (first >= 0 && regionList[first]) {
        // Region one opens holding the draft, an added one empty.
        regionList[first].words = firstRegionWords(regionList[first].words, words, first === 0 ? initial : '');
      }
    }
  }
  // With regions painted, every painted region needs its words; with none,
  // the words edit the whole picture.
  const regionsReady = $derived(
    regionList.every((r, k) => !paintedRegions[k] || (r.words ?? '').trim().length > 0),
  );
  const canSend = $derived(
    !busy && !encoding && (multi && painted ? regionsReady : words.trim().length > 0),
  );
  // Work is paint or words: an eraser tap on nothing is neither (review of #429).
  const dirty = $derived(
    editDirty({ multi, painted, words, initial, regionWords: regionList.map((r) => r.words) }),
  );

  function loaded() {
    // The canvases work at a capped size, not the photo's own: the server
    // resamples the mask and the index to its ~1024 edit canvas anyway, and
    // at a phone photo's full size four regions held ~390 MB of canvas, past
    // what iOS Safari keeps, which then reads back blank with no error
    // (review of #623). Points map to this size like any other.
    natural = workSize(img.naturalWidth, img.naturalHeight);
    size = defaultBrush(natural.width);
    base = document.createElement('canvas');
    preview = document.createElement('canvas');
    tint = document.createElement('canvas');
    for (const c of [base, view, preview, tint]) {
      c.width = natural.width;
      c.height = natural.height;
    }
    if (multi) {
      // The four colours least present in the picture, from a small copy.
      const probe = document.createElement('canvas');
      probe.width = 64;
      probe.height = Math.max(1, Math.round((64 * natural.height) / natural.width));
      const pc = probe.getContext('2d');
      pc.drawImage(img, 0, 0, probe.width, probe.height);
      palette = pickColours(pc.getImageData(0, 0, probe.width, probe.height).data);
    } else {
      palette = [{ colour: null, rgb: [255, 255, 255] }];
    }
    // In regions mode the chat's draft is the first region's words, never
    // dropped once a region is painted (review of #623).
    regionList = [{ ...palette[0], words: multi ? initial : '' }];
    // Single mode paints `base` alone; only regions keep a layer each.
    layers = multi ? [makeLayer()] : [];
    active = 0;
    render();
  }

  function makeLayer() {
    const c = document.createElement('canvas');
    c.width = natural.width;
    c.height = natural.height;
    return c;
  }

  // The union of the regions, which the dimming cuts out and `painted` reads.
  function composeBase() {
    const ctx = base.getContext('2d');
    ctx.clearRect(0, 0, base.width, base.height);
    for (const l of layers) ctx.drawImage(l, 0, 0);
  }

  function layerHasPaint(layer) {
    const data = layer.getContext('2d').getImageData(0, 0, layer.width, layer.height).data;
    for (let i = 3; i < data.length; i += 4) if (data[i] > 127) return true;
    return false;
  }

  function addRegion() {
    if (!multi || busy || regionList.length >= MAX_REGIONS) return;
    const used = new Set(regionList.map((r) => r.colour));
    const next = palette.find((c) => !used.has(c.colour));
    if (!next) return;
    regionList = [...regionList, { ...next, words: '' }];
    layers = [...layers, makeLayer()];
    active = regionList.length - 1;
  }

  function removeRegion(k) {
    if (busy || regionList.length < 2) return;
    ops = opsWithout(ops, k);
    regionList = regionList.filter((_, i) => i !== k);
    layers = layers.filter((_, i) => i !== k);
    // The brush stays on the region it was on (review of #623).
    if (k < active) active -= 1;
    active = Math.min(active, regionList.length - 1);
    replay();
    repaint();
    render();
  }

  function draw(ctx, op) {
    ctx.save();
    ctx.globalCompositeOperation = op.erase ? 'destination-out' : 'source-over';
    if (op.kind === 'box') {
      ctx.fillStyle = '#fff';
      ctx.fillRect(op.x, op.y, op.w, op.h);
    } else {
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

  // One operation onto the layers. A pixel belongs to one region: paint
  // takes it from every other region, as the index the server reads lets a
  // later region win, and the eraser clears it from all of them, so what is
  // shown painted is what is sent (review of #623).
  function apply(op) {
    layers.forEach((l, k) =>
      draw(l.getContext('2d'), op.erase || k === (op.region ?? 0) ? op : { ...op, erase: true }),
    );
  }

  // An operation as it lands: paint only grows the union, so it goes onto
  // `base` directly rather than recompositing the whole picture per pointer
  // event (review of #429). Only a regions-mode erase, which can take paint
  // out of several layers, composes them again.
  function commit(op) {
    if (multi) apply(op);
    if (multi && op.erase) composeBase();
    else draw(base.getContext('2d'), op);
  }

  function replay() {
    if (!multi) {
      base.getContext('2d').clearRect(0, 0, base.width, base.height);
      for (const op of ops) draw(base.getContext('2d'), op);
      return;
    }
    for (const l of layers) l.getContext('2d').clearRect(0, 0, l.width, l.height);
    for (const op of ops) apply(op);
    composeBase();
  }

  // The view: the picture dimmed everywhere but what is painted, which also
  // takes a faint wash of the accent so a pale area still reads as chosen.
  function render() {
    if (!view || !base) return;
    const ctx = view.getContext('2d');
    const { width: w, height: h } = view;
    ctx.clearRect(0, 0, w, h);
    const any = painted || (live && (live.kind === 'box' || !live.erase));
    if (!any) return;
    let mask = base;
    if (live?.kind === 'box') {
      mask = preview;
      const m = mask.getContext('2d');
      m.clearRect(0, 0, w, h);
      m.drawImage(base, 0, 0);
      draw(m, live);
    }
    ctx.fillStyle = 'rgba(18, 20, 31, 0.62)';
    ctx.fillRect(0, 0, w, h);
    ctx.globalCompositeOperation = 'destination-out';
    ctx.drawImage(mask, 0, 0);
    ctx.globalCompositeOperation = 'source-over';
    const t = tint.getContext('2d');
    // Each region in its own colour; a single region in the accent.
    const washes = multi
      ? layers.map((l, k) => [l, `rgb(${regionList[k].rgb.join(',')})`, 0.38])
      : [[mask, 'rgb(181, 171, 252)', 0.18]];
    if (multi && live?.kind === 'box') {
      const m = preview.getContext('2d');
      m.clearRect(0, 0, w, h);
      m.drawImage(layers[active], 0, 0);
      draw(m, live);
      washes[active] = [preview, washes[active][1], 0.38];
    }
    for (const [layer, colour, alpha] of washes) {
      t.globalCompositeOperation = 'source-over';
      t.clearRect(0, 0, w, h);
      t.fillStyle = colour;
      t.fillRect(0, 0, w, h);
      t.globalCompositeOperation = 'destination-in';
      t.drawImage(layer, 0, 0);
      ctx.globalAlpha = alpha;
      ctx.drawImage(tint, 0, 0);
      ctx.globalAlpha = 1;
    }
  }

  function point(e) {
    return toNatural(e.clientX, e.clientY, view.getBoundingClientRect(), natural);
  }

  // One pointer draws at a time: a second finger, or a thumb resting on the
  // glass, would otherwise take over the stroke in flight, draw a line
  // between the two, and leave the first stroke's dot out of the undo
  // history (review of #429).
  let drawing = null;

  function down(e) {
    if (!natural || busy || live || e.button > 0) return;
    drawing = e.pointerId;
    view.setPointerCapture(e.pointerId);
    start = point(e);
    if (tool === 'box') {
      live = { kind: 'box', region: active, ...boxFrom(start, start) };
    } else {
      live = { kind: 'stroke', region: active, erase: tool === 'erase', size, points: [start] };
      commit(live);
    }
    render();
  }

  function move(e) {
    if (!live || e.pointerId !== drawing) return;
    const p = point(e);
    if (live.kind === 'box') {
      live = { kind: 'box', region: live.region, ...boxFrom(start, p) };
    } else {
      const last = live.points[live.points.length - 1];
      live.points.push(p);
      commit({ ...live, points: [last, p] });
    }
    render();
  }

  function up(e) {
    if (!live || e.pointerId !== drawing) return;
    const op = live;
    live = null;
    drawing = null;
    if (op.kind === 'box' && (op.w < 2 || op.h < 2)) {
      render();
      return;
    }
    ops = [...ops, op];
    if (op.kind === 'box') commit(op);
    repaint();
    render();
  }

  function undo() {
    if (!ops.length || busy) return;
    ops = ops.slice(0, -1);
    replay();
    repaint();
    render();
  }

  function clear() {
    if (busy) return;
    ops = [];
    replay();
    repaint();
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

  // The regions the server reads: black everywhere, each painted region in
  // its exact colour, at the picture's own size, no smoothing.
  function indexBlob(ks) {
    const w = base.width;
    const h = base.height;
    const data = indexPixels(
      w,
      h,
      ks.map((k) => layers[k].getContext('2d').getImageData(0, 0, w, h).data),
      ks.map((k) => regionList[k].rgb),
    );
    const out = document.createElement('canvas');
    out.width = w;
    out.height = h;
    out.getContext('2d').putImageData(new ImageData(data, w, h), 0, 0);
    return new Promise((resolve) => out.toBlob(resolve, 'image/png'));
  }

  async function send() {
    if (!canSend || encoding) return;
    encoding = true;
    encodeError = null;
    try {
      if (multi && painted) {
        const ks = regionList.map((_, k) => k).filter((k) => paintedRegions[k]);
        const index = await indexBlob(ks);
        if (!index) {
          encodeError = 'The painted regions could not be prepared. Nothing was sent; try again.';
          return;
        }
        onsend?.({
          text: words,
          mask: index,
          regions: ks.map((k) => ({ colour: regionList[k].colour, words: regionList[k].words.trim() })),
        });
        return;
      }
      const mask = painted ? await maskBlob() : null;
      // A painted area whose mask could not be made must stop here: sent as
      // "nothing painted", it would redraw the whole picture the owner
      // painted a region to protect (review of #429).
      if (painted && !mask) {
        encodeError = 'The painted area could not be prepared. Nothing was sent; try again.';
        return;
      }
      onsend?.({ text: words, mask });
    } finally {
      encoding = false;
    }
  }

  function close() {
    if (!busy) onclose?.();
  }

  function keydown(e) {
    if (e.key === 'Escape') {
      // The scrim's rule: an untouched modal closes, paint is work. Cancel
      // always closes (review of #429).
      e.preventDefault();
      if (!dirty) close();
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
      {#if multi}
        Paint each thing to change in its own colour and say what changes there. Everything
        dimmed stays exactly as it is. Paint nothing to edit the whole picture.
      {:else if painted}
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

  {#if multi && regionList.length}
    <div class="regions" role="group" aria-label="Regions">
      {#each regionList as r, k (r.colour)}
        <span class="chip" class:on={k === active}>
          <button
            type="button"
            class="swatch"
            style={`--c: rgb(${r.rgb.join(',')})`}
            aria-pressed={k === active}
            aria-label={`Paint region ${k + 1}, ${r.colour}`}
            onclick={() => (active = k)}
            disabled={busy}>{k + 1}</button
          >
          {#if regionList.length > 1}
            <button type="button" class="drop" aria-label={`Remove region ${k + 1}`} onclick={() => removeRegion(k)} disabled={busy}>×</button>
          {/if}
        </span>
      {/each}
      {#if regionList.length < MAX_REGIONS}
        <button type="button" class="quiet add" onclick={addRegion} disabled={busy}>+ Region</button>
      {/if}
    </div>
  {/if}

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

  {#if multi && painted}
    <div class="regionwords">
      {#each regionList as r, k (r.colour)}
        {#if paintedRegions[k]}
          <label class="rw">
            <span class="dot" style={`--c: rgb(${r.rgb.join(',')})`}>{k + 1}</span>
            <input
              type="text"
              bind:value={regionList[k].words}
              placeholder={`What changes in ${r.colour}? e.g. make it red`}
              disabled={busy}
            />
          </label>
        {/if}
      {/each}
    </div>
  {:else}
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
  {/if}

  {#if error || encodeError}
    <p class="editerr" role="alert">{error ?? encodeError}</p>
  {/if}

  <footer class="editfoot">
    <button type="button" class="quiet" onclick={close} disabled={busy}>Cancel</button>
    <button type="button" class="go" onclick={send} disabled={!canSend}>
      {busy
        ? 'Sending…'
        : multi && painted
          ? `Edit ${paintedRegions.filter(Boolean).length > 1 ? 'painted regions' : 'painted region'}`
          : painted
            ? 'Edit painted area'
            : 'Edit whole picture'}
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
  .regions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 2px;
    border: 1px solid transparent;
    border-radius: 999px;
  }
  .chip.on {
    border-color: var(--accent-300);
  }
  .swatch,
  .dot {
    display: inline-grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 999px;
    background: var(--c);
    color: #000;
    font: 600 13px var(--sans);
    border: 2px solid rgba(255, 255, 255, 0.85);
  }
  .swatch {
    cursor: pointer;
  }
  .drop {
    padding: 0 6px;
    font: 500 16px var(--sans);
    color: var(--text-muted);
    background: none;
    border: 0;
    cursor: pointer;
  }
  .regionwords {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .rw {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .rw input {
    flex: 1;
    min-width: 0;
    padding: 9px 12px;
    font: 15px var(--sans);
    color: var(--text);
    background: var(--surface);
    border: 1px solid var(--accent-700);
    border-radius: var(--radius);
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
