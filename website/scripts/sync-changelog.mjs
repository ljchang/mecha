// The changelog has one source of truth: CHANGELOG.md at the repository root,
// which is what a reader browsing the repo (or a release tool) looks at. This
// generates the docs-site copy from it at build time, so the two cannot drift.
// The generated page is gitignored on purpose — committing it would recreate
// exactly the duplication this avoids.

import {readFileSync, writeFileSync, mkdirSync} from 'node:fs';
import {dirname, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const source = resolve(here, '../../CHANGELOG.md');
const target = resolve(here, '../docs/changelog.md');

let body;
try {
  body = readFileSync(source, 'utf8');
} catch (error) {
  console.error(`sync-changelog: cannot read ${source}: ${error.message}`);
  process.exit(1);
}

// Drop the leading "# Changelog" heading: Docusaurus renders the front-matter
// title as the page's h1, and a second one would show up twice.
body = body.replace(/^#\s+Changelog\s*\n+/, '');

const frontMatter = [
  '---',
  'title: Changelog',
  'sidebar_position: 5',
  'description: Release history for mecha, following Keep a Changelog and Semantic Versioning.',
  '---',
  '',
  '{/* Generated from CHANGELOG.md at the repository root. Do not edit here. */}',
  '',
].join('\n');

mkdirSync(dirname(target), {recursive: true});
writeFileSync(target, frontMatter + body);

console.log(`sync-changelog: wrote ${target}`);
