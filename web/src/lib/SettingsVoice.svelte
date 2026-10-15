<script>
  import { apiFetch as fetch } from './api.js';
  import { readVoicePrefs, writeVoicePrefs } from '../../../scripts/voice/voice-core.js';
  import { voicesUrl, chatVoices } from './library.js';

  // The voice pane: how calls sound, and whether the worker is up — its
  // health explains why a picker is empty, so it stays beside the pickers.
  // The voices themselves (recording, previews, who uses which) live in
  // Library → Voices since 2026-10-01.
  let voice = $state(null);

  // The voice preference and the worker's last speed range, from
  // voice-core's own store — the same bytes a connecting call reads, so a
  // choice made here is the choice the next call opens with.
  let vprefs = $state(readVoicePrefs());
  let vSavedNote = $state(null);

  // The voices to pick from are the voice library's, read now (owner,
  // 2026-10-15: one voice system for the chat and the personas). Served
  // with no unlock, so a locked voice is never offered here; a clone
  // recorded a moment ago is, without waiting for a call.
  let library = $state(null); // { names, error } once asked
  async function loadVoices() {
    try {
      const res = await fetch(voicesUrl(null));
      if (!res.ok) throw new Error((await res.text()).trim() || `HTTP ${res.status}`);
      const data = await res.json();
      // The lock first: it is why nothing would be offered.
      library = { names: chatVoices(data), error: data.locks_error ?? data.list_error ?? null };
    } catch (e) {
      library = { names: [], error: String(e?.message ?? e) };
    }
  }

  async function load() {
    try {
      const res = await fetch('/api/settings/voice');
      if (res.ok) voice = await res.json();
    } catch {
      voice = null; // unknown, shown as a dash — never as "down"
    }
  }
  load();
  loadVoices();
  // Rewritten once, so a list an older page cached on this device goes:
  // voice-core no longer keeps one (review of #701).
  writeVoicePrefs({});

  // The remembered voice when the list does not offer it — locked, or gone
  // since — is kept and not named: the select shows a dash rather than
  // either the name or a voice the next call would not open with.
  // Never a voice name: names have no spaces.
  const UNLISTED = ' unlisted';
  let unlisted = $derived(!!vprefs.voice && !!library && !library.names.includes(vprefs.voice));

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
  <!-- The rate needs no voice list, so an unreadable one never takes it
       away (review of #701). -->
  <div class="card fields">
    {#if library?.names.length}
      <label class="vfield">
        <span class="flabel">voice</span>
        <select
          class="vpick"
          value={unlisted ? UNLISTED : (vprefs.voice ?? '')}
          onchange={(e) => saveVoicePref({ voice: e.currentTarget.value })}
        >
          {#if !vprefs.voice}<option value="">worker default</option>{/if}
          {#if unlisted}<option value={UNLISTED} disabled>—</option>{/if}
          {#each library.names as v}
            <option value={v}>{v}</option>
          {/each}
        </select>
      </label>
    {:else if library}
      <div class="sub">
        {library.error
          ? `The voice list could not be read — ${library.error}.`
          : 'No voices to pick from yet — add one in Library → Voices.'}
      </div>
    {/if}
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
