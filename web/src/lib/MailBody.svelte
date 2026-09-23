<script>
  // One message body, rendered from mail-markdown.js's tree. Text nodes and
  // a closed set of elements only — no `{@html}` — because the body is a
  // stranger's text; see the header of mail-markdown.js for the link and
  // image rules. Shared by the desk, the phone and the outbox, so a thread
  // reads the same wherever it is opened.
  import { parseBlocks } from './mail-markdown.js';

  let { text = '', compact = false } = $props();
  const blocks = $derived(parseBlocks(text));
</script>

{#snippet inline(nodes)}{#each nodes as n}{#if n.t === 'text'}{n.v}{:else if n.t === 'br'}<br />{:else if n.t === 'strong'}<strong>{@render inline(n.c)}</strong>{:else if n.t === 'em'}<em>{@render inline(n.c)}</em>{:else if n.t === 'code'}<code>{n.v}</code>{:else if n.t === 'img'}<span class="img" title={n.alt || 'image not loaded'}>image{n.alt ? `: ${n.alt}` : ''}</span>{:else if n.t === 'link'}<a href={n.href} title={n.href} target="_blank" rel="noopener noreferrer nofollow">{@render inline(n.c)}</a>{/if}{/each}{/snippet}

{#snippet block(bs)}
  {#each bs as b}
    {#if b.type === 'p'}
      <p>{@render inline(b.inline)}</p>
    {:else if b.type === 'h'}
      <p class="h h{Math.min(b.level, 3)}">{@render inline(b.inline)}</p>
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

<div class="mailbody" class:compact>{@render block(blocks)}</div>

<style>
  .mailbody { font-size: 14px; line-height: 1.6; color: #d6d6de; overflow-wrap: anywhere; display: flex; flex-direction: column; gap: 10px; }
  .mailbody.compact { font-size: 13px; line-height: 1.55; gap: 8px; color: var(--text-muted); }
  .mailbody :global(p) { margin: 0; }
  .h { font-weight: 600; color: var(--text); }
  .h1 { font-size: 17px; }
  .h2 { font-size: 15px; }
  .h3 { font-size: 14px; }
  .mailbody :global(strong) { font-weight: 600; color: var(--text); }
  .mailbody :global(a) { color: var(--accent-300, #b9a8ff); text-decoration: underline; text-decoration-color: rgba(185, 168, 255, 0.35); text-underline-offset: 2px; }
  .mailbody :global(a:hover) { text-decoration-color: currentColor; }
  .mailbody :global(code) { font-family: var(--mono); font-size: 12px; padding: 0 4px; border-radius: 3px; background: var(--surface); }
  ul, ol { margin: 0; padding-left: 22px; display: flex; flex-direction: column; gap: 3px; }
  blockquote { margin: 0; padding: 2px 0 2px 12px; border-left: 2px solid #3a3a4a; color: var(--text-muted); display: flex; flex-direction: column; gap: 8px; }
  hr { width: 100%; border: 0; border-top: 1px solid #2a2a38; margin: 4px 0; }
  pre { margin: 0; font-family: var(--mono); font-size: 12px; white-space: pre-wrap; padding: 8px 10px; border-radius: var(--radius-chip); background: var(--void); }
  .img { font-family: var(--mono); font-size: 11px; padding: 0 5px; border: 1px dashed #3a3a4a; border-radius: 4px; color: var(--text-muted); }
</style>
