<script>
  // Library → Voices (owner, 2026-10-01): every voice the TTS can speak,
  // each with a spoken sample, whether it is a clone on this box, and which
  // personas speak in it — and the recorder that adds one. A persona names a
  // voice from here in its `voice` setting (PERSONA-DESIGN §11).
  //
  // Three sources, each said when it cannot be read rather than shown as an
  // empty list: the worker's list, the clones on disk, the persona store
  // (behind the library's lock, as the personas are).
  import { apiFetch as fetch } from './api.js';
  import { readVoicePrefs, writeVoicePrefs } from '../../../scripts/voice/voice-core.js';
  import VoiceRecorder from './VoiceRecorder.svelte';
  import { voicesUrl, voiceLine } from './library.js';

  let { token = null, oncount = () => {} } = $props();

  let data = $state(null);
  let error = $state(null);
  let playing = $state(null); // { name, state: 'loading' | 'playing' }
  let deleteArmed = $state(null);
  let audio = null;

  async function load() {
    try {
      const res = await fetch(voicesUrl(token));
      if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      data = await res.json();
      error = null;
      oncount(data.voices.length);
    } catch (e) {
      error = `could not read the voices: ${e?.message ?? e}`;
      oncount(null);
    }
  }
  // Again when the unlock changes: "used by" names a locked persona only
  // for an unlock.
  $effect(() => {
    void token;
    load();
  });

  function stop() {
    audio?.pause();
    if (audio?.src) URL.revokeObjectURL(audio.src);
    audio = null;
    playing = null;
  }

  async function play(name) {
    if (playing?.name === name) return stop();
    stop();
    playing = { name, state: 'loading' };
    try {
      const res = await fetch(`/api/library/voices/sample?name=${encodeURIComponent(name)}`);
      if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      const blob = await res.blob();
      if (playing?.name !== name) return;
      audio = new Audio(URL.createObjectURL(blob));
      audio.onended = stop;
      playing = { name, state: 'playing' };
      await audio.play();
    } catch (e) {
      if (playing?.name === name) playing = null;
      error = `${name}: ${e?.message ?? e}`;
    }
  }

  async function remove(name) {
    if (deleteArmed !== name) {
      deleteArmed = name;
      return;
    }
    deleteArmed = null;
    try {
      const res = await fetch('/api/settings/voice/clone/delete', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name }),
      });
      if (!res.ok) throw new Error((await res.text()).trim());
      const prefs = readVoicePrefs();
      if (prefs.voices?.includes(name)) {
        writeVoicePrefs({ voices: prefs.voices.filter((v) => v !== name) });
      }
      await load();
    } catch (e) {
      error = `${name}: ${e?.message ?? e}`;
    }
  }

  $effect(() => () => stop());
</script>

{#if error}
  <div class="warnline">{error}</div>
{/if}
{#if data?.list_error}
  <div class="warnline">The voice worker's list could not be read — {data.list_error}. Clones on disk are shown.</div>
{/if}
{#if data?.cloned_error}
  <div class="warnline">The voices directory could not be listed: {data.cloned_error}</div>
{/if}

{#if data}
  {#each data.voices as v (v.name)}
    <div class="card vrow">
      <button
        class="playbtn"
        class:on={playing?.name === v.name}
        aria-label={playing?.name === v.name ? `stop ${v.name}` : `hear ${v.name}`}
        title={v.listed === false ? 'the voice server does not list this one yet' : 'hear a sample'}
        disabled={v.listed === false}
        onclick={() => play(v.name)}
      >
        {#if playing?.name === v.name && playing.state === 'loading'}
          <span class="dots" aria-hidden="true">…</span>
        {:else if playing?.name === v.name}
          <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" aria-hidden="true"><rect x="7" y="7" width="10" height="10" rx="1.5" /></svg>
        {:else}
          <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" aria-hidden="true"><path d="M8 5v14l11-7z" /></svg>
        {/if}
      </button>
      <div class="vtext">
        <span class="vname">{v.name}</span>
        <span class="readingline">{voiceLine(v)}</span>
      </div>
      {#if v.cloned}
        <button class="abtn tiny" class:armed={deleteArmed === v.name} onclick={() => remove(v.name)}>
          {deleteArmed === v.name ? 'sure?' : 'Delete'}
        </button>
      {/if}
    </div>
  {:else}
    <div class="empty">No voices yet{data.list_error ? ' that this page could see' : ''}.</div>
  {/each}

  {#if data.cloning}
    <div class="kicker">Add a voice</div>
    <VoiceRecorder onsaved={load} />
  {:else}
    <div class="barnote">
      Recording a voice needs <code>[web] voices_dir</code> — the directory the TTS reads its
      references from — and a restart of serve.
    </div>
  {/if}
  <div class="barnote">
    A persona speaks in one of these when its settings name it: <code>voice = "name"</code>, with
    <code>voice_speed</code> if it should talk faster or slower.
  </div>
{:else if !error}
  <div class="empty">Loading…</div>
{/if}

<style>
  .card { display: flex; align-items: center; gap: 12px; padding: 10px 12px; background: var(--bg); border: 1px solid var(--accent-900); border-radius: 10px; }
  .vtext { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .vname { font-family: var(--mono); font-size: 13px; color: var(--text); overflow-wrap: anywhere; }
  .readingline { font-size: 12px; color: var(--text-muted); line-height: 1.4; }
  .playbtn { flex-shrink: 0; width: 40px; height: 40px; border-radius: 50%; display: grid; place-items: center; background: var(--surface); border: 1px solid var(--accent-700); color: var(--accent-300); cursor: pointer; }
  .playbtn.on { border-color: var(--accent-400); }
  .playbtn:disabled { opacity: 0.4; cursor: default; }
  .dots { font-family: var(--mono); }
  .abtn.tiny { flex-shrink: 0; min-height: 32px; padding: 0 10px; font-size: 11.5px; background: var(--surface); border: 1px solid var(--accent-900); border-radius: var(--radius-chip); color: var(--text-muted); cursor: pointer; }
  .abtn.tiny.armed { border-color: var(--hazard); color: var(--hazard); }
  .kicker { margin-top: 10px; font-family: var(--mono); font-size: 11px; color: var(--text-muted); text-transform: uppercase; letter-spacing: 0.06em; }
  .barnote { font-size: 12px; color: var(--text-muted); line-height: 1.45; }
  .warnline { color: var(--hazard); font-size: 12.5px; font-family: var(--mono); white-space: pre-wrap; }
  .empty { color: var(--text-muted); font-size: 13px; padding: 12px 0; }
  code { font-family: var(--mono); font-size: 11.5px; color: var(--accent-300); }
</style>
