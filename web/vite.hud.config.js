import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// The standalone dashboard bundle (`npm run build:hud`): one `hud.js` and one
// `hud.css`, Vega inlined, for a page that serves a spec and its datasets
// beside it — the factory's `hud` template vendors these
// (`docs/LIVE-DASHBOARD-DESIGN.md` §4.1, §6.1). CSS is a file, never injected,
// so the strictest `style-src` holds.
export default defineConfig({
  plugins: [svelte()],
  // The app's public files (its favicon, the voice uplink's transform) are not
  // the dashboard's.
  publicDir: false,
  build: {
    minify: true,
    outDir: 'dist-hud',
    emptyOutDir: true,
    lib: {
      entry: 'src/hud-standalone.js',
      formats: ['es'],
      fileName: () => 'hud.js',
      cssFileName: 'hud',
    },
    rollupOptions: { output: { inlineDynamicImports: true } },
  },
});
