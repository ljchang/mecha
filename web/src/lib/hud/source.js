// Where a standalone dashboard's datasets come from, and how a failed read is
// told apart (`docs/LIVE-DASHBOARD-DESIGN.md` §6.2, R16). Plain JS so node can
// test it; `hud-standalone.js` is the page that uses it.

/** Where this page's datasets are, from its own path. */
export function dataBase(pathname) {
  const version = /^(\/b\/[a-z0-9_-]+\/)v\/\d+\//.exec(pathname);
  if (version) return `${version[1]}data/`;
  const grant = /^(\/g\/[0-9a-f]+\/)/.exec(pathname);
  if (grant) return `${grant[1]}data/`;
  return './data/';
}

/** Datasets beside the page, revalidated by ETag. */
export function filesSource(base, { granted = false } = {}) {
  let seen = false;
  return {
    async dataset(name, etag) {
      const res = await fetch(`${base}${encodeURIComponent(name)}.json`, {
        cache: 'no-cache',
        headers: etag ? { 'If-None-Match': etag } : {},
      });
      if (res.status === 304) return null;
      if (!res.ok) {
        // A grant lasts an hour and is re-checked on every fetch: once it
        // has ended, the data that was readable a minute ago answers 404.
        // The page keeps the numbers it has, marked stale, and says how to
        // get current ones (R16) — never an empty dashboard.
        if (granted && seen && (res.status === 404 || res.status === 410)) {
          const e = new Error('your viewing access has ended — sign in again for current numbers');
          e.signIn = true;
          throw e;
        }
        throw new Error(`HTTP ${res.status}`);
      }
      seen = true;
      return { dataset: await res.json(), etag: res.headers.get('ETag') };
    },
  };
}
