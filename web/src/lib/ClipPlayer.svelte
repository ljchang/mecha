<script>
  // A clip under its tool row (docs/BACKGROUND-JOBS-DESIGN.md §8). Fetched
  // only when the owner presses play: the transcript renders whole, and a
  // clip, unlike a picture, can be tens of megabytes, so a long chat must not
  // pull every clip in its scrollback at mount (review of #666, pass 4).
  // Fetched as a blob and played from an object URL: the chat's file route
  // keeps serving only pictures inline, and a phone needs no range requests.
  // The URL is revoked when the clip changes or the row goes.
  let { src } = $props();
  let wanted = $state(false);
  let url = $state(null);
  let failed = $state(false);

  $effect(() => {
    const at = src;
    if (!wanted) return;
    let made = null;
    let live = true;
    url = null;
    failed = false;
    fetch(at)
      .then(async (r) => {
        if (!r.ok) throw new Error(String(r.status));
        const blob = new Blob([await r.arrayBuffer()], { type: 'video/mp4' });
        if (!live) return;
        made = URL.createObjectURL(blob);
        url = made;
      })
      .catch(() => {
        if (live) failed = true;
      });
    return () => {
      live = false;
      if (made) URL.revokeObjectURL(made);
    };
  });
</script>

{#if url}
  <!-- svelte-ignore a11y_media_has_caption -->
  <video class="genclip" src={url} controls playsinline autoplay></video>
{:else if failed}
  <span class="clipnote">the clip could not be loaded</span>
  <button class="clipplay" onclick={() => { wanted = false; wanted = true; }}>Try again</button>
{:else if wanted}
  <span class="clipnote">loading the clip…</span>
{:else}
  <button class="clipplay" onclick={() => (wanted = true)}>▶ Play the clip</button>
{/if}

<style>
  .genclip {
    display: block;
    max-width: 100%;
    max-height: 70vh;
    margin: 6px 0;
    border-radius: 8px;
    background: #000;
  }
  .clipnote {
    display: block;
    margin: 4px 0;
    font-size: 0.85em;
    opacity: 0.7;
  }
  .clipplay {
    display: block;
    margin: 6px 0;
    padding: 10px 14px;
    border-radius: 8px;
    border: 1px solid currentColor;
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
</style>
