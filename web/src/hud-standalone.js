// The standalone dashboard (`docs/LIVE-DASHBOARD-DESIGN.md` §4.1): the same
// `Dashboard` the app draws at `#hud/<id>`, built as one self-contained
// `hud.js` + `hud.css` (`npm run build:hud`) for a page that serves the spec
// and its datasets beside it:
//
//   <link rel="stylesheet" href="hud.css">
//   <div id="hud"></div>
//   <script type="module" src="hud.js"></script>
//
// with `hud.json` beside it. Nothing here names another origin: every fetch
// is on the page's own.
//
// **Where the data is** depends on how the page was reached (§6.2): a
// dashboard's rows live beside its versions, never inside one, so on the
// factory a page at `/b/<id>/v/<n>/` reads `/b/<id>/data/`, and one opened
// under a viewing grant at `/g/<cap>/…` reads `/g/<cap>/data/`. Anywhere else
// (a local preview) the data sits beside the page, in `./data/`.
import { mount } from 'svelte';
import Dashboard from './lib/hud/Dashboard.svelte';
import './hud-page.css';
import { dataBase, filesSource } from './lib/hud/source.js';

// No web font is bundled: a published page carries no font bytes, so the
// stack falls back to `system-ui` where the app has Inter loaded.

async function boot() {
  const target = document.getElementById('hud');
  if (!target) return;
  const base = './';
  try {
    const res = await fetch(`${base}hud.json`, { cache: 'no-cache' });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const spec = await res.json();
    if (typeof spec.title === 'string') document.title = spec.title;
    const data = dataBase(location.pathname);
    const source = filesSource(data, { granted: data.startsWith('/g/') });
    mount(Dashboard, { target, props: { spec, source, mode: 'auto' } });
  } catch (e) {
    target.textContent = `This dashboard could not be loaded: ${e?.message ?? e}`;
  }
}

boot();
