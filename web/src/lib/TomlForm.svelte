<script>
  // A form over a TOML file the owner writes (`mecha-core/src/tomlform.rs`),
  // drawn from the server's schema: sections of typed fields. Any file with a
  // schema gets this page — nothing here knows which file it is.
  //
  // The draft lives with the caller (`bind:draft`) so it survives a switch to
  // another file and back; `onsave` receives only the changed fields.
  import { untrack } from 'svelte';
  import './form.css';
  import { draftOf, changesOf, segmented, chipsFor, toggleChip, addChip } from './tomlform.js';

  let { form, values, draft = $bindable(), busy = false, onsave, onclose } = $props();

  // The first values only, on purpose: the caller re-keys this component on
  // the file's digest, so new values mean a new form, not a moving draft.
  if (!draft) draft = untrack(() => draftOf(form, values));
  let typing = $state({});

  const changes = $derived(changesOf(form, values, draft));
  const count = $derived(Object.keys(changes).length);

  function changed(path) {
    return Object.hasOwn(changes, path);
  }

  function discard() {
    draft = draftOf(form, values);
    typing = {};
  }

  function add(path) {
    draft[path] = addChip(draft[path], typing[path]);
    typing[path] = '';
  }

  function optionHelp(field) {
    return field.options.find((o) => o.value === draft[field.path])?.help ?? null;
  }
</script>

<div class="tf">
  {#each form.sections as section (section.title)}
    <section class="tf-sec">
      <header>
        <h3>{section.title}</h3>
        {#if section.help}<p class="tf-help">{section.help}</p>{/if}
      </header>
      {#each section.fields as field (field.path)}
        {@const stack = field.kind === 'chips' || field.kind === 'text'}
        <div class="tf-row" class:stack class:unbuilt={field.unbuilt || section.unbuilt}>
          <div class="tf-label">
            <span class="tf-name">
              {field.label}
              {#if changed(field.path)}<span class="tf-dot" title="changed" aria-label="changed"></span>{/if}
            </span>
            {#if field.help}<span class="tf-help">{field.help}</span>{/if}
          </div>

          {#if field.kind === 'toggle'}
            <button
              class="tf-switch"
              role="switch"
              aria-checked={draft[field.path]}
              aria-label={field.label}
              disabled={busy}
              onclick={() => (draft[field.path] = !draft[field.path])}
            ><span class="tf-knob"></span></button>
          {:else if field.kind === 'choice' && segmented(field)}
            <div class="tf-seg" role="radiogroup" aria-label={field.label}>
              {#each field.options as o (o.value)}
                <button
                  role="radio"
                  aria-checked={draft[field.path] === o.value}
                  class:on={draft[field.path] === o.value}
                  disabled={busy}
                  onclick={() => (draft[field.path] = o.value)}
                >{o.label}</button>
              {/each}
            </div>
          {:else if field.kind === 'choice'}
            <div class="tf-select">
              <select
                aria-label={field.label}
                disabled={busy}
                value={draft[field.path] ?? ''}
                onchange={(e) => (draft[field.path] = e.currentTarget.value || null)}
              >
                {#if field.none}<option value="">{field.none}</option>{/if}
                {#each field.options as o (o.value)}<option value={o.value}>{o.label}</option>{/each}
                {#if draft[field.path] && !field.options.some((o) => o.value === draft[field.path])}
                  <option value={draft[field.path]}>{draft[field.path]}</option>
                {/if}
              </select>
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 9l6 6 6-6" /></svg>
            </div>
          {:else if field.kind === 'chips'}
            <div class="tf-chips">
              {#each chipsFor(field, draft[field.path]) as c (c.value)}
                {@const on = draft[field.path].includes(c.value)}
                <button
                  class="tf-chip"
                  class:on
                  aria-pressed={on}
                  disabled={busy}
                  onclick={() => (draft[field.path] = toggleChip(draft[field.path], c.value))}
                >
                  {#if c.extra}
                    <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
                  {:else if on}
                    <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M5 12.5l4.5 4.5L19 7.5" /></svg>
                  {/if}
                  {c.extra ? c.label : c.label.replaceAll('_', ' ')}
                </button>
              {/each}
              {#if field.free}
                <form class="tf-add" onsubmit={(e) => { e.preventDefault(); add(field.path); }}>
                  <input
                    placeholder="add"
                    autocapitalize="off"
                    autocorrect="off"
                    spellcheck="false"
                    disabled={busy}
                    bind:value={typing[field.path]}
                  />
                  <button type="submit" aria-label="add" disabled={busy || !typing[field.path]?.trim()}>
                    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M12 5v14M5 12h14" /></svg>
                  </button>
                </form>
              {:else if !field.options.length}
                <span class="tf-help">None to choose from yet.</span>
              {/if}
            </div>
          {:else if field.kind === 'text'}
            <input
              class="tf-text"
              aria-label={field.label}
              placeholder={field.placeholder ?? ''}
              maxlength={field.max}
              disabled={busy}
              bind:value={draft[field.path]}
            />
          {/if}
          {#if field.kind === 'choice' && optionHelp(field)}
            <span class="tf-help tf-opthelp">{optionHelp(field)}</span>
          {/if}
        </div>
      {/each}
    </section>
  {/each}

  <div class="tf-bar" class:live={count > 0}>
    {#if onclose}<button class="tf-btn ghost" onclick={onclose}>Close</button>{/if}
    <span class="tf-count">{#if count}<span class="tf-dot"></span>{count} change{count === 1 ? '' : 's'}{/if}</span>
    {#if count}<button class="tf-btn" disabled={busy} onclick={discard}>Discard</button>{/if}
    <button class="tf-btn primary" disabled={busy || !count} onclick={() => onsave(changes)}>Save</button>
  </div>
</div>
