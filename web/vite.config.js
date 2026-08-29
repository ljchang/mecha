import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Dev proxies /api to a locally running `mecha serve`; the production build is
// static files that `mecha serve` serves itself.
//
// `--mode demo` builds the same app against `src/demo/` instead of a box, for
// the documentation site to embed. It differs in exactly two ways, both here:
// the fixture transport is switched on, and the base is relative, because the
// docs serve it from `/demo/` while `mecha serve` serves it from `/`.
export default defineConfig(({ mode }) => ({
  plugins: [svelte()],
  base: mode === 'demo' ? './' : '/',
  define: {
    'import.meta.env.VITE_MECHA_DEMO': JSON.stringify(mode === 'demo'),
  },
  server: {
    proxy: { '/api': 'http://127.0.0.1:7643' },
  },
  build: {
    outDir: mode === 'demo' ? 'dist-demo' : 'dist',
    emptyOutDir: true,
  },
}));
