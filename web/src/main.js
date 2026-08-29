import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';

// The documentation build (`npm run build:demo`) answers `/api` from fixtures
// so the docs site can embed the real app with no box behind it.
//
// Two properties this shape is chosen for. `VITE_MECHA_DEMO` is an
// `import.meta.env` constant, so a normal build folds the branch to `false`
// and Rollup drops the dynamic import — the demo code is not in the bundle
// mecha serves, which `npm run check-demo` proves rather than assumes. And the
// install is awaited *before* `mount`, because the components fetch on
// creation: installing after would race the first `/api` call.
async function boot() {
  if (import.meta.env.VITE_MECHA_DEMO) {
    const { installDemo } = await import('./demo/index.js');
    installDemo();
  }
  return mount(App, { target: document.getElementById('app') });
}

export default boot();
