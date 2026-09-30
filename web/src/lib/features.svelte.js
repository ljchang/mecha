// The page's one reading of `/api/features`, shared by the nav, Home, Chat,
// Dictate and Settings (the rules are `features.js`'s).
//
// Read when the app opens and each time it comes back into view: the route
// re-reads the global config per request, so a `mecha features enable`
// reaches a page on its next look, with no restart (FEATURES-DESIGN.md §4.2).
import { apiFetch as fetch } from './api.js';
import { index } from './features.js';

// `body` undefined is "not asked yet"; `error` set is "could not ask" — and
// either way every surface is shown (`isShown` with no rows).
export const features = $state({ body: undefined, rows: null, error: null });

export async function loadFeatures() {
  try {
    const res = await fetch('/api/features');
    if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
    const body = await res.json();
    features.body = body;
    features.rows = index(body);
    features.error = null;
  } catch (e) {
    // A failed re-read is unknown, not the previous answer left standing:
    // show everything and say why, rather than hide off a stale read.
    features.body = null;
    features.rows = null;
    features.error = String(e?.message ?? e);
  }
}
