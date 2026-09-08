// A custom header forces cross-origin mutations through a CORS preflight.
// mecha serve grants no CORS access. This is intent, not authentication:
// the Tailscale owner check still applies to every request.
export function apiFetch(path, options = {}) {
  if (typeof path !== 'string' || !path.startsWith('/api/')) {
    throw new TypeError('apiFetch requires a same-origin /api/ path');
  }
  const headers = new Headers(options.headers);
  const method = (options.method ?? 'GET').toUpperCase();
  if (!['GET', 'HEAD', 'OPTIONS'].includes(method)) {
    headers.set('X-Mecha-Request', '1');
  }
  return globalThis.fetch(path, { ...options, headers, mode: 'same-origin' });
}
