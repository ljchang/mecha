<script>
  import { apiFetch as fetch } from './api.js';
  import PictureSettings from './PictureSettings.svelte';

  // The chat's picture settings (the owner, 2026-10-11): what a picture in
  // any chat is drawn with, and what a persona draws with for each setting
  // its own page leaves at "chat". Kept on the server, read fresh by every
  // picture, so a change here is the next picture's on every surface.
  let view = $state(null);
  let error = $state(null);
  let busy = $state(false);

  async function load() {
    try {
      const res = await fetch('/api/settings/images');
      if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      view = await res.json();
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    }
  }
  load();

  // Each change saves at once, as the persona page's do.
  async function save(body) {
    busy = true;
    try {
      const res = await fetch('/api/settings/images', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(body),
      });
      if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      const j = await res.json();
      view = { ...view, image: j.image };
      error = null;
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
    }
  }
</script>

<p class="hint">
  How pictures are drawn in chat. A persona draws with these too, for each setting its own page leaves at
  "chat". The LoRAs every picture in chat carries are chosen under Settings → LoRAs, behind the library lock.
</p>
{#if error}<p class="err">{error}</p>{/if}
{#if view}
  <PictureSettings img={view.image} opts={view.image_options} {busy} {save} />
{:else if !error}
  <p class="hint">—</p>
{/if}

<style>
  .hint { font-size: 12px; color: var(--text-muted); margin: 0 0 12px; }
  .err { font-size: 12px; color: var(--danger, #e66); margin: 0 0 12px; }
</style>
