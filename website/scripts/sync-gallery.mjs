// The component gallery is generated in mecha-factory (`cargo run --example
// gallery`) and committed there, because that is where the renderer lives and
// therefore where a drift check can fail the right build. This copies the
// committed output into `static/` so the docs pages can embed it.
//
// Two sources, in order:
//
//   1. A sibling checkout at ../../../mecha-factory/gallery — what a developer
//      with both repos open has, and the only one that shows uncommitted work.
//   2. The public tarball of mecha-factory's default branch, for CI and for
//      anyone who only cloned mecha.
//
// The copy is gitignored. Committing it here would make two repositories the
// source of truth for the same bytes, and the second one would be the stale
// one — the exact duplication sync-changelog.mjs exists to avoid.
//
// A missing gallery is a **warning, not an error**. Prose is most of these
// pages and it should still build offline; an iframe with nothing behind it is
// visible on the page that has one, which is a better failure than no docs.

import {cpSync, existsSync, mkdirSync, rmSync} from 'node:fs';
import {dirname, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {mkdtempSync} from 'node:fs';
import {join} from 'node:path';

const TARBALL = 'https://codeload.github.com/ljchang/mecha-factory/tar.gz/refs/heads/main';

const here = dirname(fileURLToPath(import.meta.url));
const sibling = resolve(here, '../../../mecha-factory/gallery');
const target = resolve(here, '../static/factory/gallery');

function install(from) {
  rmSync(target, {recursive: true, force: true});
  mkdirSync(dirname(target), {recursive: true});
  cpSync(from, target, {recursive: true});
}

if (existsSync(sibling)) {
  install(sibling);
  console.log(`sync-gallery: copied ${sibling} → ${target}`);
} else {
  // curl and tar rather than a dependency: the docs site has no runtime need
  // for an HTTP client, and adding one for a build step is how a package.json
  // grows things nobody can account for.
  const scratch = mkdtempSync(join(tmpdir(), 'mecha-gallery-'));
  try {
    execFileSync('bash', ['-c', `curl -fsSL "${TARBALL}" | tar -xz -C "${scratch}"`], {
      stdio: ['ignore', 'ignore', 'pipe'],
    });
    const extracted = join(scratch, 'mecha-factory-main', 'gallery');
    if (!existsSync(extracted)) {
      throw new Error('the tarball has no gallery/ directory');
    }
    install(extracted);
    console.log(`sync-gallery: fetched mecha-factory@main → ${target}`);
  } catch (error) {
    console.warn(
      `sync-gallery: no gallery available (${error.message.trim()}). ` +
        'The pages that embed it will render empty frames. ' +
        'Clone ljchang/mecha-factory beside this repository to fix it locally.',
    );
  } finally {
    rmSync(scratch, {recursive: true, force: true});
  }
}
