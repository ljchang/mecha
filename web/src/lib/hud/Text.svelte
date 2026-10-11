<script>
  // A text panel. Markdown through mail-markdown.js's parser — text nodes and
  // a closed set of elements, no `{@html}` — with **no links**: a link renders
  // as its words and its destination is dropped, and an image as its alt
  // text. The spec already refuses link syntax (§2.2); this is the second
  // layer behind it, so a destination the server screen missed has nowhere
  // to land.
  import { parseBlocks } from '../mail-markdown.js';

  let { markdown = '' } = $props();
  const blocks = $derived(parseBlocks(markdown));
</script>

{#snippet inline(nodes)}{#each nodes as n}{#if n.t === 'text'}{n.v}{:else if n.t === 'br'}<br />{:else if n.t === 'strong'}<strong>{@render inline(n.c)}</strong>{:else if n.t === 'em'}<em>{@render inline(n.c)}</em>{:else if n.t === 'code'}<code>{n.v}</code>{:else if n.t === 'img'}{n.alt ?? ''}{:else if n.t === 'link'}{@render inline(n.c)}{/if}{/each}{/snippet}

{#snippet block(bs)}
  {#each bs as b}
    {#if b.type === 'p'}
      <p>{@render inline(b.inline)}</p>
    {:else if b.type === 'h'}
      <p class="h">{@render inline(b.inline)}</p>
    {:else if b.type === 'ul'}
      <ul>{#each b.items as it}<li>{@render inline(it)}</li>{/each}</ul>
    {:else if b.type === 'ol'}
      <ol start={b.start}>{#each b.items as it}<li>{@render inline(it)}</li>{/each}</ol>
    {:else if b.type === 'quote'}
      <blockquote>{@render block(b.blocks)}</blockquote>
    {:else if b.type === 'hr'}
      <hr />
    {:else if b.type === 'code'}
      <pre>{b.text}</pre>
    {/if}
  {/each}
{/snippet}

<div class="text">{@render block(blocks)}</div>

<style>
  .text { font-size: 14px; line-height: 1.6; color: var(--hud-text); display: flex; flex-direction: column; gap: 8px; overflow-wrap: anywhere; }
  .text :global(p) { margin: 0; }
  .h { font-weight: 600; }
  .text :global(code), pre { font-family: 'JetBrains Mono', ui-monospace, monospace; font-size: 12px; }
  pre { margin: 0; white-space: pre-wrap; }
  ul, ol { margin: 0; padding-left: 20px; }
  blockquote { margin: 0; padding-left: 10px; border-left: 2px solid var(--hud-line); color: var(--hud-muted); }
  hr { width: 100%; border: 0; border-top: 1px solid var(--hud-line); margin: 2px 0; }
</style>
