<script>
  // A form over a Markdown file the owner writes (`mecha-core/src/mdform.rs`):
  // a title and its note, text, and `## ` sections, each with a note and its
  // text. The server writes the file back from these in one layout, so every
  // file saved here reads the same. `fixed` names sections that must stay.
  //
  // Drawn as one page, not a stack of cards: the writing is the content, and
  // each section's controls wait behind one ⋯ menu (design critique of the
  // first cut — the chrome repeated more often than the text did).
  import { untrack } from 'svelte';
  import './form.css';
  import { draftOf, docOf, isDirty, isFixed, move, addSection, problems, MAX_HEADING } from './mdform.js';

  let {
    doc,
    fixed = [],
    draft = $bindable(),
    busy = false,
    labels = {},
    onsave,
    onclose,
  } = $props();

  // The first doc only: the caller re-keys this component on the file's
  // digest, so a new doc means a new form.
  if (!draft) draft = untrack(() => draftOf(doc, fixed));

  const dirty = $derived(isDirty(doc, draft));
  const wrong = $derived(problems(draft, fixed));
  // Text before the sections: always there for a file without sections (a
  // motivation is mostly this), otherwise once there is some or it is asked for.
  let showBody = $state(untrack(() => Boolean(draft.body.trim()) || draft.sections.length === 0));
  // Notes open now: every note the file already has, plus any the owner
  // opened. Held here, not read off the text, so emptying a note keeps its
  // box and the focus (review of #430).
  const openNotes = (d) => new Set([...(d.note.trim() ? ['title'] : []), ...d.sections.filter((s) => s.note.trim()).map((s) => s.id)]);
  let notes = $state(untrack(() => openNotes(draft)));
  let menu = $state(null);

  const noteOpen = (key) => notes.has(key);

  function openNote(key) {
    notes = new Set([...notes, key]);
    menu = null;
  }

  function act(fn) {
    fn();
    menu = null;
  }

  function discard() {
    draft = draftOf(doc, fixed);
    notes = openNotes(draft);
    menu = null;
  }

  // Grow a text box with its text, so a section reads as a page rather than
  // a scrolling well. It refits on typing, on a width change (rotation, a
  // box first laid out while hidden) and when its value is set from outside
  // (Discard, a restored draft); and it holds the scroll still while it
  // measures, or collapsing to `auto` jumps the page on iOS (owner,
  // 2026-10-01: "buggy on mobile"). The scroller is whichever ancestor
  // actually scrolls, found by its style — a class name would tie this to
  // the page that hosts the form (review of #457).
  function scrollerOf(node) {
    for (let el = node.parentElement; el; el = el.parentElement) {
      const y = getComputedStyle(el).overflowY;
      if (y === 'auto' || y === 'scroll') return el;
    }
    return document.scrollingElement;
  }
  function autosize(node, _value) {
    const scroller = scrollerOf(node);
    let width = 0;
    const fit = () => {
      const top = scroller?.scrollTop ?? 0;
      node.style.height = 'auto';
      node.style.height = `${node.scrollHeight}px`;
      if (scroller) scroller.scrollTop = top;
    };
    requestAnimationFrame(fit);
    node.addEventListener('input', fit);
    const seen = new ResizeObserver(([entry]) => {
      const w = Math.round(entry.contentRect.width);
      if (w !== width) {
        width = w;
        fit();
      }
    });
    seen.observe(node);
    return {
      update: () => requestAnimationFrame(fit),
      destroy: () => {
        node.removeEventListener('input', fit);
        seen.disconnect();
      },
    };
  }

  // Menu icons, as SVG paths.
  const NOTE = 'M4 20h4L19 9l-4-4L4 16v4zM13.5 6.5l4 4';
  const UP = 'M6 15l6-6 6 6';
  const DOWN = 'M6 9l6 6 6-6';
  const REMOVE = 'M5 7h14M10 7V5h4v2M7 7l1 12h8l1-12';
  const TEXT = 'M5 7h14M5 12h14M5 17h9';

  function outside(e) {
    if (menu != null && !e.target.closest?.('.tf-menuwrap')) menu = null;
  }
</script>

<svelte:window onclick={outside} onkeydown={(e) => e.key === 'Escape' && (menu = null)} />

{#snippet dots(key, label)}
  <button
    class="tf-more"
    aria-label={label}
    aria-haspopup="menu"
    aria-expanded={menu === key}
    disabled={busy}
    onclick={() => (menu = menu === key ? null : key)}
  >
    <svg viewBox="0 0 24 24" width="18" height="18" fill="currentColor" aria-hidden="true"><circle cx="5" cy="12" r="1.6" /><circle cx="12" cy="12" r="1.6" /><circle cx="19" cy="12" r="1.6" /></svg>
  </button>
{/snippet}

{#snippet item(label, run, icon, danger = false, off = false)}
  <button role="menuitem" class="tf-item" class:danger disabled={off} onclick={() => act(run)}>
    <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={icon} /></svg>
    {label}
  </button>
{/snippet}

{#snippet note(key, value, set)}
  {#if noteOpen(key, value)}
    <div class="tf-note">
      <textarea
        class="tf-area tf-notearea"
        rows="1"
        placeholder="A note to yourself — never sent to a chat"
        aria-label="Note to yourself"
        disabled={busy}
        {value}
        oninput={(e) => set(e.currentTarget.value)}
        use:autosize={value}
      ></textarea>
    </div>
  {/if}
{/snippet}

<div class="tf">
  <article class="tf-page">
    <header class="tf-part tf-top">
      <div class="tf-headrow">
        <input
          class="tf-title"
          aria-label="Title"
          placeholder={labels.title ?? 'Title'}
          maxlength={MAX_HEADING}
          disabled={busy}
          bind:value={draft.title}
        />
        {#if !noteOpen('title', draft.note) || !showBody}
          <div class="tf-menuwrap">
            {@render dots('title', 'More for the title')}
            {#if menu === 'title'}
              <div class="tf-menu" role="menu">
                {#if !noteOpen('title', draft.note)}{@render item('Add a note', () => openNote('title'), NOTE)}{/if}
                {#if !showBody}{@render item('Add text before the sections', () => (showBody = true), TEXT)}{/if}
              </div>
            {/if}
          </div>
        {/if}
      </div>
      {@render note('title', draft.note, (v) => (draft.note = v))}
      {#if showBody}
        <textarea
          class="tf-area tf-body"
          rows="1"
          placeholder={labels.body ?? 'Write here'}
          aria-label={labels.body ?? 'Text'}
          disabled={busy}
          bind:value={draft.body}
          use:autosize={draft.body}
        ></textarea>
      {/if}
    </header>

    {#each draft.sections as s, i (s.id)}
      {@const pinned = isFixed(fixed, s)}
      <section class="tf-part">
        <div class="tf-headrow">
          <span class="tf-hash" aria-hidden="true">##</span>
          {#if pinned}
            <span class="tf-heading tf-pinned">
              {s.heading}
              <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-label="always kept"><path d="M7 11V7a5 5 0 0110 0v4M5 11h14v10H5z" /></svg>
            </span>
          {:else}
            <input class="tf-heading" aria-label="Section heading" placeholder="Heading" maxlength={MAX_HEADING} disabled={busy} bind:value={s.heading} />
          {/if}
          <div class="tf-menuwrap">
            {@render dots(s.id, `More for ${s.heading || 'this section'}`)}
            {#if menu === s.id}
              <div class="tf-menu" role="menu">
                {#if !noteOpen(s.id, s.note)}{@render item('Add a note', () => openNote(s.id), NOTE)}{/if}
                {@render item('Move up', () => (draft.sections = move(draft.sections, i, -1)), UP, false, i === 0)}
                {@render item('Move down', () => (draft.sections = move(draft.sections, i, 1)), DOWN, false, i === draft.sections.length - 1)}
                {#if !pinned}
                  {@render item('Remove section', () => (draft.sections = draft.sections.filter((x) => x.id !== s.id)), REMOVE, true)}
                {/if}
              </div>
            {/if}
          </div>
        </div>
        {#if pinned && labels.fixed?.[s.heading.trim()]}
          <p class="tf-caption">{labels.fixed[s.heading.trim()]}</p>
        {/if}
        {@render note(s.id, s.note, (v) => (s.note = v))}
        <textarea class="tf-area tf-body" rows="1" placeholder="Write here" aria-label={s.heading || 'Section text'} disabled={busy} bind:value={s.body} use:autosize={s.body}></textarea>
      </section>
    {/each}

    <button class="tf-addsec" disabled={busy} onclick={() => (draft.sections = addSection(draft.sections))}>
      <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg>
      Add a section
    </button>
  </article>

  {#each wrong as w}<div class="tf-wrong">{w}</div>{/each}

  <div class="tf-bar" class:live={dirty}>
    {#if onclose}<button class="tf-btn ghost" onclick={onclose}>Close</button>{/if}
    <span class="tf-count">{#if dirty}<span class="tf-dot"></span>Unsaved{/if}</span>
    {#if dirty}<button class="tf-btn" disabled={busy} onclick={discard}>Discard</button>{/if}
    <button class="tf-btn primary" disabled={busy || !dirty || wrong.length > 0} onclick={() => onsave(docOf(draft))}>Save</button>
  </div>
</div>
