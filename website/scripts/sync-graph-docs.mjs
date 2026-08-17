// mecha-graph's docs have one source of truth: the mecha-graph repository.
// This copies four of its files into the site's Graph section at build time,
// so the site can never drift from the repo — the exact duplication
// sync-changelog.mjs exists to avoid, applied across a repository boundary
// the way sync-gallery.mjs already does.
//
// Two sources, in order:
//
//   1. A sibling checkout at ../../../personalized_knowledge_graph — what a
//      developer with both repos open has. Its shipped docs are byte-equal
//      to the public repo's (the export gate guarantees it).
//   2. raw.githubusercontent.com from the public repo, for CI and for anyone
//      who only cloned mecha.
//
// The copies are gitignored. A missing file is a warning, not an error —
// the authored overview still builds, and a sidebar entry that 404s is a
// better failure than no docs build at all.

import {mkdirSync, readFileSync, writeFileSync, existsSync} from 'node:fs';
import {dirname, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';

const RAW = 'https://raw.githubusercontent.com/ljchang/mecha-graph/main';
const here = dirname(fileURLToPath(import.meta.url));
const sibling = resolve(here, '../../../personalized_knowledge_graph');
const outDir = resolve(here, '../docs/graph');

const FILES = [
  {
    src: 'docs/ARCHITECTURE.md',
    out: 'architecture.md',
    title: 'Architecture',
    position: 2,
    description:
      'Episodes are evidence, nodes are things, facts are beliefs, the context pack is the product.',
  },
  {
    src: 'docs/INTEGRATIONS.md',
    out: 'integrations.md',
    title: 'Integrations',
    position: 3,
    description: 'Per-source auth and configuration, and the at-rest design.',
  },
  {
    src: 'docs/PLAN.md',
    out: 'self-improvement.md',
    title: 'Self-improvement',
    position: 4,
    description:
      'The doctrine, the gossip roles, the mechanism catalog, and the build order.',
  },
  {
    src: 'CHANGELOG.md',
    out: 'changelog.md',
    title: 'Changelog',
    position: 9,
    description: 'Release history for mecha-graph.',
  },
];

// Repo-relative links become section-relative ones; a link to the README
// becomes a link to the repository, which is where the README lives.
function rewriteLinks(text) {
  return text
    .replaceAll('docs/ARCHITECTURE.md', './architecture')
    .replaceAll('docs/INTEGRATIONS.md', './integrations')
    .replaceAll('docs/PLAN.md', './self-improvement')
    .replaceAll('README.md', 'https://github.com/ljchang/mecha-graph');
}

async function fetchSource(src) {
  const local = resolve(sibling, src);
  if (existsSync(local)) return readFileSync(local, 'utf8');
  const response = await fetch(`${RAW}/${src}`);
  if (!response.ok) throw new Error(`${response.status} for ${RAW}/${src}`);
  return await response.text();
}

mkdirSync(outDir, {recursive: true});
for (const file of FILES) {
  try {
    let text = await fetchSource(file.src);
    // The frontmatter title replaces a leading H1; keeping both renders the
    // heading twice.
    text = text.replace(/^# .*\n/, '');
    text = rewriteLinks(text);
    const front = [
      '---',
      `title: ${file.title}`,
      `sidebar_position: ${file.position}`,
      `description: ${file.description}`,
      '---',
      '',
      `{/* Synced from mecha-graph (${file.src}) at build time. Do not edit here. */}`,
      '',
    ].join('\n');
    writeFileSync(resolve(outDir, file.out), front + text);
    console.log(`sync-graph-docs: ${file.src} → docs/graph/${file.out}`);
  } catch (error) {
    console.warn(`sync-graph-docs: skipped ${file.src}: ${error.message}`);
  }
}
