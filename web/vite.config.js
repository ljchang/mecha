import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Dev proxies /api to a locally running `mecha serve`; the production build is
// static files that `mecha serve` serves itself.
export default defineConfig({
  plugins: [svelte()],
  server: {
    proxy: { '/api': 'http://127.0.0.1:7643' },
  },
  build: { outDir: 'dist', emptyOutDir: true },
});
