// The dashboard theme (`docs/LIVE-DASHBOARD-DESIGN.md` §4.4).
//
// A spec names a theme and never carries a colour: a model choosing hex
// values is how every generated dashboard ends up looking the same. The page's
// colours are CSS custom properties in Dashboard.svelte's stylesheet, and the
// same palette reaches Vega here as a `config`, so a chart and the panel
// around it agree in light and in dark.
//
// One theme ships built in, `default`. Owner-written themes
// (`~/.mecha/hud/themes/<name>.toml`) are a later step; until then every
// spec draws in this one.

const FONT = 'Inter, system-ui, sans-serif';

const PALETTES = {
  dark: {
    text: '#e9e9ed',
    muted: '#9a9aa8',
    grid: '#2c2e3c',
    domain: '#44465a',
    category: ['#8fb4ff', '#f2a65a', '#7fd1ae', '#e88a9a', '#b9a8ff', '#e8d46a', '#6cc6d9', '#c49a7a', '#a0a8b8'],
  },
  light: {
    text: '#1d1f2b',
    muted: '#5d6070',
    grid: '#e3e4ea',
    domain: '#b8bac6',
    category: ['#3b6fd6', '#d9822b', '#2f9e74', '#c94f64', '#7a5ccf', '#b59a1c', '#2a93a8', '#946a4c', '#6b7280'],
  },
};

/** `light` or `dark`; anything else is dark, the app's own scheme. */
export function scheme(mode) {
  return mode === 'light' ? 'light' : 'dark';
}

/**
 * The Vega `config` for a scheme. `narrow` is a chart a phone's width: its
 * legend wraps into columns rather than running off the edge.
 */
export function vegaConfig(mode, narrow = false) {
  const p = PALETTES[scheme(mode)];
  const axis = {
    labelColor: p.muted,
    titleColor: p.muted,
    gridColor: p.grid,
    domainColor: p.domain,
    tickColor: p.domain,
    labelFont: FONT,
    titleFont: FONT,
    labelFontSize: 11,
    titleFontSize: 11,
    titleFontWeight: 500,
    // Labels that would collide are dropped, never drawn over each other.
    labelOverlap: 'greedy',
  };
  return {
    background: null,
    font: FONT,
    view: { stroke: null },
    axis,
    legend: {
      labelColor: p.muted,
      titleColor: p.muted,
      labelFont: FONT,
      titleFont: FONT,
      labelFontSize: 11,
      titleFontSize: 11,
      orient: 'bottom',
      symbolType: 'circle',
      ...(narrow ? { columns: 3, labelLimit: 90 } : {}),
    },
    // A time axis on a phone asks for fewer ticks: its labels are wide, and a
    // chart filling its container lays them out before the overlap pass.
    axisTemporal: { labelSeparation: 6, ...(narrow ? { tickCount: 4 } : {}) },
    header: { labelColor: p.muted, titleColor: p.text, labelFont: FONT, titleFont: FONT },
    title: { color: p.text, font: FONT, fontWeight: 500 },
    range: { category: p.category },
    mark: { color: p.category[0] },
    line: { strokeWidth: 2 },
    area: { fillOpacity: 0.85, line: false },
    bar: { cornerRadiusEnd: 2 },
  };
}

/** The theme names this build can draw. */
export const THEMES = ['default'];
