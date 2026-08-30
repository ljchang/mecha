<script>
  // Is the loop improving anything? Three views over `learning-report --json`.
  //
  // **The honest framing is part of the component, not a footnote.** These
  // series are observational over one owner's real work: the task mix moves
  // under the metric, so a falling correction rate may mean better rules or
  // an easier week. The caveat ships in the payload and renders below the
  // charts, because a trend line is exactly the shape that reads as proof.
  //
  // Inline SVG rather than a chart library: three small charts do not earn a
  // dependency, and the CSP on this page allows no external script anyway.
  let { report } = $props();

  const W = 560;
  const H = 150;
  const PAD = { l: 34, r: 8, t: 8, b: 22 };
  const plotW = W - PAD.l - PAD.r;
  const plotH = H - PAD.t - PAD.b;

  // Only buckets that actually measured something can carry a rate. A bucket
  // with no sessions has `rate: null` from the server — never 0.0 — and
  // drawing it as zero would show a perfect week where there was no week.
  const rated = $derived((report?.buckets ?? []).filter((b) => b.rate != null));

  const maxRate = $derived(Math.max(0.01, ...rated.map((b) => b.rate)));
  const x = (i, n) => (n <= 1 ? PAD.l + plotW / 2 : PAD.l + (i / (n - 1)) * plotW);
  const yRate = (v) => PAD.t + plotH - (v / maxRate) * plotH;

  const linePts = $derived(
    rated.map((b, i) => `${x(i, rated.length)},${yRate(b.rate)}`).join(' ')
  );

  // Error-type composition, stacked. Sorted by total so the legend order is
  // stable between renders rather than following whatever the map iterated.
  const types = $derived.by(() => {
    const totals = new Map();
    for (const b of report?.buckets ?? []) {
      for (const [k, v] of Object.entries(b.error_types ?? {})) {
        totals.set(k, (totals.get(k) ?? 0) + v);
      }
    }
    return [...totals.entries()].sort((a, b) => b[1] - a[1]).map(([k]) => k);
  });

  const PALETTE = [
    'var(--c1)', 'var(--c2)', 'var(--c3)',
    'var(--c4)', 'var(--c5)', 'var(--c6)',
  ];

  const maxCount = $derived(
    Math.max(1, ...(report?.buckets ?? []).map((b) => b.reflections))
  );

  // Stacking is precomputed rather than accumulated during render. The
  // shorter spelling — a mutable `{@const}` running `y -= h` inside the
  // inner each — depends on template expressions evaluating in source order
  // and on that accumulator being fresh per bar, neither of which is a
  // guarantee worth resting a chart on.
  const bars = $derived.by(() => {
    const bs = report?.buckets ?? [];
    if (bs.length === 0) return [];
    const slot = plotW / bs.length;
    const bw = Math.max(6, slot - 6);
    return bs.map((b, i) => {
      let y = PAD.t + plotH;
      const segs = [];
      types.forEach((t, ti) => {
        const c = b.error_types?.[t] ?? 0;
        if (c <= 0) return;
        const h = (c / maxCount) * plotH;
        y -= h;
        segs.push({ t, c, h, y, fill: PALETTE[ti % PALETTE.length] });
      });
      return { period: b.period, x: PAD.l + (i + 0.5) * slot - bw / 2, bw, segs };
    });
  });

  const steps = $derived(report?.steps ?? []);
  const maxRules = $derived(
    Math.max(1, ...steps.flatMap((s) => [s.rules_before, s.rules_after]))
  );

  const health = $derived(Object.entries(report?.health ?? {}));
  const short = (p) => (p ?? '').slice(5);
</script>

{#if !report}
  <p class="muted">Loading…</p>
{:else if rated.length === 0}
  <p class="muted">
    No sessions recorded yet — the correction rate needs a denominator before it means anything.
  </p>
{:else}
  <section class="chart">
    <h4>Correction rate</h4>
    <p class="sub">
      Reflections per session. Falling is the loop working — read it with the caveat below.
    </p>
    <svg viewBox="0 0 {W} {H}" role="img" aria-label="Correction rate over time">
      {#each [0, 0.5, 1] as t}
        <line
          class="grid"
          x1={PAD.l} x2={W - PAD.r}
          y1={yRate(maxRate * t)} y2={yRate(maxRate * t)}
        />
        <text class="ax y" x={PAD.l - 6} y={yRate(maxRate * t)}>
          {(maxRate * t).toFixed(2)}
        </text>
      {/each}
      <polyline class="line" points={linePts} />
      {#each rated as b, i (b.period)}
        <circle class="dot" cx={x(i, rated.length)} cy={yRate(b.rate)} r="3">
          <title>{b.period}: {b.rate.toFixed(2)} ({b.reflections} reflections / {b.sessions} sessions)</title>
        </circle>
        <text class="ax x" x={x(i, rated.length)} y={H - 6}>{short(b.period)}</text>
      {/each}
    </svg>
  </section>

  <section class="chart">
    <h4>What needed correcting</h4>
    <p class="sub">Reflections by error type — the composition, not just the count.</p>
    <svg viewBox="0 0 {W} {H}" role="img" aria-label="Error types over time">
      {#each bars as bar (bar.period)}
        {#each bar.segs as seg (seg.t)}
          <rect x={bar.x} width={bar.bw} y={seg.y} height={seg.h} fill={seg.fill}>
            <title>{bar.period} · {seg.t}: {seg.c}</title>
          </rect>
        {/each}
        <text class="ax x" x={bar.x + bar.bw / 2} y={H - 6}>{short(bar.period)}</text>
      {/each}
    </svg>
    <ul class="legend">
      {#each types as t, ti (t)}
        <li><i style="background:{PALETTE[ti % PALETTE.length]}"></i>{t}</li>
      {/each}
    </ul>
  </section>

  {#if steps.length > 0}
    <section class="chart">
      <h4>Rule set over time</h4>
      <p class="sub">Every consolidation and retirement pass, and what it left live.</p>
      <ul class="steps">
        {#each steps.slice().reverse() as s (s.at + s.domain)}
          <li>
            <span class="when">{(s.at ?? '').slice(0, 10)}</span>
            <span class="dom">{s.domain}</span>
            <span class="delta" class:down={s.rules_after < s.rules_before}>
              {s.rules_before} → {s.rules_after}
            </span>
            <span class="muted">
              {#if s.reflections > 0}from {s.reflections} reflection(s){:else}retirement{/if}
            </span>
          </li>
        {/each}
      </ul>
    </section>
  {/if}

  <section class="chart">
    <h4>Rule health</h4>
    <ul class="health">
      {#each health as [domain, h] (domain)}
        <li>
          <b>{domain}</b>
          <span>{h.active} active</span>
          <span>{h.retired} retired</span>
          <!-- Never-validated is reported, not hidden: an entirely unmeasured
               rule set means the ledger is not running, and every claim
               downstream of it is empty. -->
          <span class:warn={h.never_validated > 0 && h.never_validated === h.active}>
            {h.never_validated} never validated
          </span>
          <span class:warn={h.attributed_regressions > 0}>
            {h.attributed_regressions} attributed regression(s)
          </span>
        </li>
      {/each}
    </ul>
  </section>

  <p class="caveat">{report.caveat}</p>
{/if}

<style>
  .chart { margin: 0 0 1.4rem; }
  h4 { margin: 0 0 0.15rem; font-size: 0.95rem; }
  .sub { margin: 0 0 0.5rem; font-size: 0.8rem; opacity: 0.7; }
  svg { width: 100%; height: auto; overflow: visible; }
  .grid { stroke: currentColor; opacity: 0.14; }
  .line { fill: none; stroke: var(--c1); stroke-width: 2; }
  .dot { fill: var(--c1); }
  .ax { font-size: 9px; fill: currentColor; opacity: 0.6; }
  .ax.y { text-anchor: end; dominant-baseline: middle; }
  .ax.x { text-anchor: middle; }
  .legend, .health, .steps { list-style: none; margin: 0.4rem 0 0; padding: 0; }
  .legend { display: flex; flex-wrap: wrap; gap: 0.1rem 0.8rem; font-size: 0.75rem; }
  .legend i { display: inline-block; width: 9px; height: 9px; border-radius: 2px; margin-right: 4px; }
  .steps li, .health li {
    display: flex; flex-wrap: wrap; gap: 0.6rem; align-items: baseline;
    font-size: 0.8rem; padding: 0.2rem 0;
    border-bottom: 1px solid color-mix(in srgb, currentColor 10%, transparent);
  }
  .when { font-variant-numeric: tabular-nums; opacity: 0.7; }
  .dom { font-weight: 600; }
  .delta { font-variant-numeric: tabular-nums; }
  .delta.down { color: var(--c3); }
  .muted { opacity: 0.65; font-size: 0.8rem; }
  .warn { color: var(--c2); }
  .caveat {
    font-size: 0.78rem; opacity: 0.75; line-height: 1.45;
    border-left: 2px solid color-mix(in srgb, currentColor 25%, transparent);
    padding-left: 0.6rem; margin: 0;
  }
  :global(:root) {
    --c1: #4f7cff; --c2: #e0793a; --c3: #3aa675;
    --c4: #a568d4; --c5: #d4526e; --c6: #6f8899;
  }
</style>
