// Build the web surface in demo mode and install it under `static/demo/`, so
// the docs pages can embed the real app rather than a picture of one.
//
// **Why this is a build step and not committed output.** `web/` is in this
// repository, so unlike the factory gallery there is no second source of truth
// to sync from — there is only a source tree and a build of it. Committing the
// bundle would mean every change to `web/src/` either regenerates a 180 kB
// diff or silently stops matching the app it claims to be. So the docs build
// builds it, and `static/demo/` is gitignored.
//
// **A missing demo is a warning, not an error**, matching `sync-gallery.mjs`:
// prose is most of these pages and it should still build with no `web/`
// node_modules on the machine. `WebFrame` renders a visible "not built" panel
// on the one page that embeds it, which is a better failure than no docs.
//
// The app is dark-only and hash-routed, which is what makes one build serve
// every page in the docs: `<WebFrame page="chat">` is a fragment, not a
// separate bundle.

import {cpSync, existsSync, mkdirSync, rmSync} from 'node:fs';
import {dirname, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';

const here = dirname(fileURLToPath(import.meta.url));
const web = resolve(here, '../../web');
const built = resolve(web, 'dist-demo');
const target = resolve(here, '../static/demo');

function run(command, args) {
  execFileSync(command, args, {cwd: web, stdio: 'inherit'});
}

try {
  if (!existsSync(resolve(web, 'node_modules'))) {
    // `npm ci` rather than `install`: the lockfile is committed and a docs
    // build has no business resolving a different dependency tree than the
    // one the app is developed against.
    console.log('build-demo: installing web/ dependencies');
    run('npm', ['ci']);
  }
  run('npm', ['run', 'build:demo']);

  rmSync(target, {recursive: true, force: true});
  mkdirSync(dirname(target), {recursive: true});
  cpSync(built, target, {recursive: true});
  console.log(`build-demo: ${built} → ${target}`);
} catch (error) {
  console.warn(
    `build-demo: could not build the web demo (${String(error.message).trim().split('\n')[0]}). ` +
      'The pages that embed it will say so. Run `npm ci && npm run build:demo` in web/ to fix it locally.',
  );
}
