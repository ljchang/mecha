<script>
  import { apiFetch as fetch } from './api.js';
  import { readVoicePrefs, writeVoicePrefs } from '../../../scripts/voice/voice-core.js';

  // The voice pane: how calls sound, and whether the worker is up — its
  // health explains why a picker is empty, so it stays beside the pickers.
  // The voices themselves (recording, previews, who uses which) live in
  // Library → Voices since 2026-10-01.
  let voice = $state(null);

  // The voice preference and the last call's voices/range, from voice-core's
  // own store — the same bytes a connecting call reads, so a choice made
  // here is the choice the next call opens with. The list and range are a
  // cache of the worker's last answer: a picker with no live call cannot
  // ask, and rendering the remembered answer with a dated note beats either
  // a hardcoded list or no picker at all.
  let vprefs = $state(readVoicePrefs());
  let vSavedNote = $state(null);

  async function load() {
    try {
      const res = await fetch('/api/settings/voice');
      if (res.ok) voice = await res.json();
    } catch {
      voice = null; // unknown, shown as a dash — never as "down"
    }
  }
  load();

  function saveVoicePref(patch) {
    writeVoicePrefs(patch);
    vprefs = readVoicePrefs();
    vSavedNote = 'saved — applies from the next call';
  }

</script>

<p class="hint">
  How calls sound. A choice here is what the next call opens with — a call already running
  keeps the voice it started in.
</p>

<!-- The worker first: it is what explains an empty picker below. -->
{#if voice === null}
  <div class="status"><span class="label">worker</span><span class="val">—</span></div>
{:else if voice.offer_target === null}
  <div class="status"><span class="label">worker</span><span class="val">not reached: <code>[voice] offer_target</code> is empty</span></div>
{:else}
  <div class="status">
    <span class="label">worker</span>
    <span class="val" style:color={voice.worker_reachable ? 'var(--accent-400)' : 'var(--hazard)'}>
      {voice.worker_reachable ? 'up' : 'unreachable'}
    </span>
    <span class="target">{voice.offer_target}</span>
  </div>
{/if}

<section>
  <div class="kicker">Playback</div>
  {#if vprefs.voices?.length}
    <div class="card fields">
      <label class="vfield">
        <span class="flabel">voice</span>
        <select
          class="vpick"
          value={vprefs.voice ?? ''}
          onchange={(e) => saveVoicePref({ voice: e.currentTarget.value })}
        >
          {#if !vprefs.voice}<option value="">worker default</option>{/if}
          {#each vprefs.voices as v}
            <option value={v}>{v}</option>
          {/each}
        </select>
      </label>
      <label class="vfield">
        <span class="flabel">rate</span>
        <!-- The bounds are the worker's own last answer, never a literal
             here: it owns what it can speak at. -->
        <input
          type="range"
          min={vprefs.range?.min ?? 0.5}
          max={vprefs.range?.max ?? 2}
          step="0.05"
          value={vprefs.speed ?? 1}
          onchange={(e) => saveVoicePref({ speed: Number(e.currentTarget.value) })}
        />
        <span class="vval">{Number(vprefs.speed ?? 1).toFixed(2)}×</span>
      </label>
      {#if vSavedNote}<div class="sub ok">{vSavedNote}</div>{/if}
    </div>
  {:else}
    <div class="card">
      <div class="sub">
        No voice list remembered yet — it arrives from the worker on the first call, and the
        pickers appear here after that.
      </div>
    </div>
  {/if}
</section>

<section>
  <div class="kicker">Voices</div>
  <!-- Moved to the library with the voices a persona can speak in (owner,
       2026-10-01): hear each one, record or upload a clone, see who uses it. -->
  <a class="card linkcard" href="#library/voices">
    <span>Every voice on this box — hear one, record or upload a new one — is in Library → Voices.</span>
    <span class="go" aria-hidden="true">→</span>
  </a>
</section>

<style>
  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: 12.5px;
    line-height: 1.45;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .status {
    display: flex;
    align-items: baseline;
    gap: 10px;
    font-size: 12.5px;
  }
  .status .label {
    font-family: var(--mono);
    font-size: 11px;
    color: var(--text-muted);
  }
  .status .val {
    font-family: var(--mono);
  }
  .status .target {
    margin-left: auto;
    font-family: var(--mono);
    font-size: 10.5px;
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card {
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .vfield {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .flabel {
    width: 44px;
    color: var(--text-muted);
    font-family: var(--mono);
    font-size: 11px;
  }
  .vpick {
    flex: 1;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--accent-700);
    border-radius: var(--radius-chip);
    font: inherit;
    font-size: 13px;
    min-height: 36px;
    padding: 5px 8px;
  }
  .vfield input[type='range'] {
    flex: 1;
    accent-color: var(--accent-400);
  }
  .vval {
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--text-muted);
    width: 44px;
    text-align: right;
  }
  .sub {
    color: var(--text-muted);
    font-size: 12px;
    line-height: 1.4;
  }
  .sub.ok {
    color: var(--accent-300);
  }
  .linkcard {
    flex-direction: row;
    align-items: center;
    color: var(--text);
    text-decoration: none;
    font-size: 13px;
    line-height: 1.45;
  }
  .linkcard .go {
    margin-left: auto;
    color: var(--accent-400);
  }
</style>
