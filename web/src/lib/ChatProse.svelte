<script>
  // A chat reply as rendered Markdown — headings, lists, bold, code — from
  // mail-markdown.js's tree, so it is safe for the same reasons the mail
  // reader is: text nodes and a closed set of elements, no `{@html}`, a link
  // only when it is http, https or mailto, and no image fetched. A reply can
  // carry a third party's words (a persona's paper, a page a tool read), and
  // a renderer that emitted HTML would run them.
  //
  // Double-click shows the reply as written, formatting codes and all, and
  // double-click again goes back (the owner's ask, 2026-10-01).
  //
  // `cites`: a persona reply's checked citations as [raw, check] pairs
  // (`citeEntries`). Each is drawn where its text falls, tagged with what the
  // check found; `onCite` opens a found one.
  //
  // `actions`: who said it (`mecha`, a persona's name), for a finished reply
  // — Copy and Download act on the reply as written, and every code block
  // gets its own Copy (owner request, 2026-10-01). Left out while a reply
  // streams: half an answer is not one to save.
  import { parseBlocks, hiddenTarget } from './mail-markdown.js';
  import { citeNote, citeOpens, citeMark, citeUnmark } from './persona.js';
  import { replyFilename, copyText, downloadText } from './reply-export.js';

  let { text = '', cites = null, onCite = null, actions = null } = $props();
  let raw = $state(false);
  // Which control just copied — 'reply' or a code block's index — for a
  // moment's "copied", or 'failed' when the phone refused.
  let copied = $state(null);
  let copiedTimer = null;
  async function copy(what, value) {
    const ok = await copyText(value);
    copied = ok ? what : 'failed';
    clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => (copied = null), 1500);
  }
  // Citations are swapped for placeholders before the Markdown is parsed
  // and drawn back from them, so the parser cannot split one (`citeMark`).
  const prepared = $derived(citeMark(text, cites));
  const blocks = $derived(parseBlocks(prepared.text));
  const pieces = (v) => citeUnmark(v, prepared.marks);
  // Code keeps a citation's words, drawn plain: a placeholder must never
  // show (review of #479).
  const plain = (v) => pieces(v).map((p) => p.text).join('');
</script>

{#snippet textnode(v)}{#each pieces(v) as seg}{#if seg.check}{@const n = citeNote(seg.check)}{#if citeOpens(seg.check) && onCite}<span class="cite {n.tone}" role="button" tabindex="0" title={n.title} onclick={() => onCite(seg.check)} onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), onCite(seg.check))}>{seg.text}<span class="citetag">{n.label}</span></span>{:else}<span class="cite {n.tone}" title={n.title}>{seg.text}<span class="citetag">{n.label}</span></span>{/if}{:else}{seg.text}{/if}{/each}{/snippet}

{#snippet inline(nodes)}{#each nodes as n}{#if n.t === 'text'}{@render textnode(n.v)}{:else if n.t === 'br'}<br />{:else if n.t === 'strong'}<strong>{@render inline(n.c)}</strong>{:else if n.t === 'em'}<em>{@render inline(n.c)}</em>{:else if n.t === 'code'}<code>{plain(n.v)}</code>{:else if n.t === 'img'}<span class="img" title={n.alt || 'image not loaded'}>image{n.alt ? `: ${n.alt}` : ''}</span>{:else if n.t === 'link'}<a href={n.href} title={n.href} target="_blank" rel="noopener noreferrer nofollow">{@render inline(n.c)}</a>{#if hiddenTarget(n)}<span class="dest">{' → '}{hiddenTarget(n)}</span>{/if}{/if}{/each}{/snippet}

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
      {#if actions}
        <div class="codewrap">
          <pre>{plain(b.text)}</pre>
          <button class="codecopy" type="button" aria-label="Copy this code" onclick={() => copy(b, plain(b.text))}>
            {copied === b ? 'copied' : 'copy'}
          </button>
        </div>
      {:else}
        <pre>{plain(b.text)}</pre>
      {/if}
    {/if}
  {/each}
{/snippet}

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="prose" class:raw ondblclick={() => (raw = !raw)} title={raw ? 'as written — double-click to render' : undefined}>
  {#if raw}{text}{:else}{@render block(blocks)}{/if}
</div>
{#if actions && text.trim()}
  <div class="replyacts">
    <button type="button" class="ract" aria-label="Copy this reply" onclick={() => copy('reply', text)}>
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="8" y="8" width="12" height="12" rx="2" /><path d="M16 8V6a2 2 0 00-2-2H6a2 2 0 00-2 2v8a2 2 0 002 2h2" /></svg>
      {copied === 'reply' ? 'Copied' : 'Copy'}
    </button>
    <button type="button" class="ract" aria-label="Download this reply as Markdown" onclick={() => downloadText(replyFilename(actions), text)}>
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 4v11M7 10l5 5 5-5M5 20h14" /></svg>
      Download
    </button>
    {#if copied === 'failed'}<span class="copyfail">this browser would not copy — double-click the reply to select it as written</span>{/if}
  </div>
{/if}

<style>
  .prose { display: flex; flex-direction: column; gap: 10px; white-space: normal; overflow-wrap: anywhere; }
  .prose.raw { display: block; white-space: pre-wrap; font-family: var(--mono); font-size: 12.5px; color: var(--text-muted); }
  .prose :global(p) { margin: 0; }
  .h { font-weight: 600; color: var(--text); }
  .h1 { font-size: 1.2em; }
  .h2 { font-size: 1.1em; }
  .h3 { font-size: 1em; }
  .prose :global(strong) { font-weight: 600; color: var(--text); }
  /* Where a link really goes, shown when its words say otherwise — a title
     a phone cannot hover is not enough (the mail reader's rule). */
  .dest { font-family: var(--mono); font-size: 0.85em; color: var(--hazard); }
  .prose :global(a) { color: var(--accent-300, #b9a8ff); text-decoration: underline; text-underline-offset: 2px; }
  .prose :global(code) { font-family: var(--mono); font-size: 0.9em; padding: 0 4px; border-radius: 3px; background: var(--surface); }
  ul, ol { margin: 0; padding-left: 22px; display: flex; flex-direction: column; gap: 4px; }
  blockquote { margin: 0; padding: 2px 0 2px 12px; border-left: 2px solid var(--accent-900); color: var(--text-muted); display: flex; flex-direction: column; gap: 8px; }
  hr { width: 100%; border: 0; border-top: 1px solid var(--accent-900); margin: 2px 0; }
  .replyacts { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; margin-top: 6px; }
  .ract { display: inline-flex; align-items: center; gap: 5px; min-height: 30px; padding: 0 9px; background: transparent; border: 1px solid transparent; border-radius: 8px; color: var(--text-muted); font-size: 12px; cursor: pointer; }
  .ract:hover { background: var(--surface); border-color: var(--accent-900); color: var(--text); }
  .copyfail { font-size: 11px; color: var(--hazard); }
  .codewrap { position: relative; }
  .codewrap pre { padding-right: 56px; }
  .codecopy { position: absolute; top: 4px; right: 4px; min-height: 26px; padding: 0 8px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: 6px; color: var(--text-muted); font-family: var(--mono); font-size: 10.5px; cursor: pointer; }
  .codecopy:hover { color: var(--text); }
  pre { margin: 0; font-family: var(--mono); font-size: 12px; white-space: pre-wrap; padding: 8px 10px; border-radius: 8px; background: var(--void); }
  .img { font-family: var(--mono); font-size: 11px; padding: 0 5px; border: 1px dashed var(--accent-900); border-radius: 4px; color: var(--text-muted); }
  /* Citations, as the persona page draws them (§10.4). */
  .cite { text-decoration: underline dotted var(--accent-500); text-underline-offset: 3px; }
  .cite[role='button'] { cursor: pointer; }
  .cite.bad { text-decoration: underline wavy var(--hazard); }
  .cite.muted { text-decoration: none; }
  .citetag { margin-left: 4px; padding: 0 5px; border-radius: 6px; font-size: 11px; vertical-align: 1px; white-space: nowrap; background: var(--surface); color: var(--text-muted); }
  .cite.ok .citetag { color: var(--accent-300); }
  .cite.warn .citetag { color: var(--text); }
  .cite.bad .citetag { color: var(--hazard); }
</style>
