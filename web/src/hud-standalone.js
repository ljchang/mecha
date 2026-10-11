// The standalone dashboard (`docs/LIVE-DASHBOARD-DESIGN.md` §4.1): the same
// `Dashboard` the app draws at `#hud/<id>`, built as one self-contained
// `hud.js` + `hud.css` (`npm run build:hud`) for a page that serves the spec
// and its datasets beside it:
//
//   <link rel="stylesheet" href="hud.css">
//   <div id="hud"></div>
//   <script type="module" src="hud.js"></script>
//
// with `hud.json` and `data/<name>.json` in the same directory. Nothing here
// names another origin: every fetch is relative to the page.
import { mount } from 'svelte';
import Dashboard from './lib/hud/Dashboard.svelte';
import './hud-page.css';

// No web font is bundled: a published page carries no font bytes, so the
// stack falls back to `system-ui` where the app has Inter loaded.

/** Datasets as files beside the page, revalidated by ETag. */
export function filesSource(base) {
  return {
    async dataset(name, etag) {
      const res = await fetch(`${base}data/${encodeURIComponent(name)}.json`, {
        cache: 'no-cache',
        headers: etag ? { 'If-None-Match': etag } : {},
      });
      if (res.status === 304) return null;
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      return { dataset: await res.json(), etag: res.headers.get('ETag') };
    },
  };
}

async function boot() {
  const target = document.getElementById('hud');
  if (!target) return;
  const base = './';
  try {
    const res = await fetch(`${base}hud.json`, { cache: 'no-cache' });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const spec = await res.json();
    if (typeof spec.title === 'string') document.title = spec.title;
    mount(Dashboard, { target, props: { spec, source: filesSource(base), mode: 'auto' } });
  } catch (e) {
    target.textContent = `This dashboard could not be loaded: ${e?.message ?? e}`;
  }
}

boot();
