<script>
  import { apiFetch as fetch } from './api.js';
  import { layerUrl, toggled, chatLine } from './loras.js';

  // One picture layer's LoRA list (docs/LORA-DESIGN.md §4, §6): the chat's
  // on Settings → LoRAs, a persona's on its page. Read and written through
  // its own route, never the picture settings' body, and only under the
  // library unlock: without `token` it shows that it is locked, and the
  // server sends no names to show. A persona's list may follow the chat's.
  let { persona = null, token = null } = $props();

  let view = $state(null);
  let error = $state(null);
  let busy = $state(false);

  // Each load is numbered, so a slower answer for the persona shown before
  // never lands over the one shown now.
  let seq = 0;
  async function load(who, t) {
    const mine = ++seq;
    view = null;
    error = null;
    if (!t) return;
    try {
      const res = await fetch(layerUrl(who, t));
      if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      const j = await res.json();
      if (mine === seq) view = j.locked ? null : j;
    } catch (e) {
      if (mine === seq) error = String(e?.message ?? e);
    }
  }
  $effect(() => {
    load(persona, token);
  });

  async function save(inherit, names) {
    busy = true;
    try {
      const res = await fetch(layerUrl(persona, null), {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ unlock: token, inherit, names }),
      });
      if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      view = await res.json();
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }

  const following = $derived(view?.layer === 'inherit' && !!persona);
  const on = $derived(following ? view?.chat ?? [] : view?.names ?? []);
  // What a save sends: the names the registry still has. A name it lost is
  // dropped on the next change rather than refused with it (review of #751).
  const kept = $derived(on.filter((n) => view?.choices.some((c) => c.name === n)));
</script>

<div class="loraset">
  <div class="head">
    LoRAs
    <span class="hint">
      {persona ? "On every picture this persona draws." : 'On every picture in chat, and for each persona that follows the chat.'}
      At most {view?.max ?? 3}; the model is not told which.
    </span>
  </div>
  {#if error}<span class="bad">{error}</span>{/if}
  {#if !token}
    <span class="hint">Behind the library lock: unlock to see or change them.</span>
  {:else if view}
    {#if view.layer === 'damaged'}
      <span class="bad">The stored list does not read, so no LoRA applies; choose again to replace it.</span>
    {/if}
    {#if persona}
      <label class="check">
        <input type="checkbox" checked={following} disabled={busy} onchange={(e) => save(e.currentTarget.checked, e.currentTarget.checked ? [] : kept)} />
        Follow the chat's list ({chatLine(view)})
      </label>
    {/if}
    {#each view.choices as c (c.name)}
      {@const checked = on.includes(c.name)}
      <label class="check" class:muted={following}>
        <input
          type="checkbox"
          {checked}
          disabled={busy || following || (!checked && kept.length >= view.max)}
          onchange={() => save(false, toggled(kept, c.name, view.max))}
        />
        <span class="name">{c.name}</span>
        <span class="hint inline" class:bad={c.status !== 'checked'}>{c.status === 'checked' ? c.about : c.reason}</span>
      </label>
    {:else}
      <span class="hint">No picture LoRAs in the registry yet: add them under Settings → LoRAs.</span>
    {/each}
    {#each on.filter((n) => !view.choices.some((c) => c.name === n)) as gone (gone)}
      <span class="bad">{gone} is on the list but no longer in the registry; the next change here drops it.</span>
    {/each}
    {#if !following && on.length > kept.length}
      <button class="drop" disabled={busy} onclick={() => save(false, kept)}>Drop {on.length - kept.length === 1 ? 'it' : 'them'} now</button>
    {/if}
  {:else if !error}
    <span class="hint">reading…</span>
  {/if}
</div>

<style>
  .loraset { display: flex; flex-direction: column; gap: 8px; padding: 12px 14px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: 10px; font-size: 14px; }
  .head { min-width: 0; }
  .hint { display: block; margin-top: 2px; font-size: 11px; color: var(--text-muted); }
  .hint.inline { display: inline; margin: 0; overflow-wrap: anywhere; }
  .check { display: flex; align-items: baseline; flex-wrap: wrap; gap: 4px 8px; font-size: 13px; }
  .check input { align-self: center; }
  .muted { opacity: 0.6; }
  .name { font-family: var(--mono); font-size: 12px; }
  .bad { color: var(--hazard); font-size: 12px; }
  .drop { align-self: flex-start; background: none; border: none; padding: 4px 0; color: var(--accent-400); font-size: 12px; cursor: pointer; }
</style>
