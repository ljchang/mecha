<script>
  // The renderer's one link sink (`docs/GITHUB-DASHBOARD-DESIGN.md` §4): a
  // GitHub reference, as an anchor only when `githubHref` builds one from its
  // fixed template, and as text otherwise. `web/test/hud.mjs` allows a link
  // here, by this file's name and this one expression, and nowhere else.
  //
  // No referrer, and no opener: GitHub is not told which board the click
  // came from, and the opened page cannot reach back into this one.
  import { githubHref, refText } from './hud.js';

  let { reference, row, linkable = null, links = false } = $props();
</script>

{#if githubHref(reference, row, linkable, links)}<a href={githubHref(reference, row, linkable, links)} target="_blank" rel="noopener noreferrer" referrerpolicy="no-referrer">{refText(reference, row)}</a>{:else}{refText(reference, row)}{/if}

<style>
  a { color: inherit; text-decoration: underline; text-underline-offset: 2px; }
</style>
